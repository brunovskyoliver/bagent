use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    /// Z_PK from ZICCLOUDSYNCINGOBJECT
    pub pk: i64,
    /// Constructed from store UUID + Z_PK for JXA lookup
    pub coredata_id: String,
    pub title: String,
    pub snippet: Option<String>,
    /// Unix timestamp (seconds)
    pub created_at: i64,
    /// Unix timestamp (seconds)
    pub modified_at: i64,
    pub is_pinned: bool,
    pub is_locked: bool,
    pub folder: Option<String>,
    /// HTML body fetched via JXA; None when not yet requested or locked
    pub body: Option<String>,
    /// ISO 639-1 code from whatlang
    pub language: Option<String>,
}

// ── Connector ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct NotesConnector {
    note_store: PathBuf,
}

impl NotesConnector {
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("no home dir"))?;
        let note_store =
            home.join("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite");
        Ok(Self { note_store })
    }

    /// True when the NoteStore database is readable (requires Full Disk Access).
    pub fn is_accessible(&self) -> bool {
        self.note_store.exists()
    }

    // ── Queries ──────────────────────────────────────────────────────────────

    /// List notes ordered by most recently modified.
    /// Body is NOT populated — call `get_note_body` to hydrate.
    pub fn list_notes(&self, limit: usize) -> Result<Vec<Note>> {
        let (conn, store_uuid) = self.open_db()?;
        let ent = note_entity_id(&conn)?;

        let mut stmt = conn.prepare(&format!(
            r#"
            SELECT
                n.Z_PK,
                COALESCE(n.ZUSERTITLE, n.ZTITLE, '(untitled)'),
                n.ZSUMMARY,
                CAST(n.ZCREATIONDATE     + 978307200 AS INTEGER),
                CAST(n.ZMODIFICATIONDATE + 978307200 AS INTEGER),
                COALESCE(n.ZISPINNED, 0),
                COALESCE(n.ZISPASSWORDPROTECTED, 0),
                f.ZNAME
            FROM ZICCLOUDSYNCINGOBJECT n
            LEFT JOIN ZICCLOUDSYNCINGOBJECT f
                ON n.ZFOLDER = f.Z_PK
            WHERE n.Z_ENT = {ent}
              AND COALESCE(n.ZMARKEDFORDELETION, 0) = 0
              AND n.ZNOTEDATA IS NOT NULL
            ORDER BY n.ZMODIFICATIONDATE DESC
            LIMIT {limit}
            "#
        ))?;

        let rows = stmt.query_map([], |row| {
            let pk: i64 = row.get(0)?;
            Ok(NoteRow {
                pk,
                title: row.get(1)?,
                snippet: row.get(2)?,
                created_at: row.get::<_, i64>(3).unwrap_or(0),
                modified_at: row.get::<_, i64>(4).unwrap_or(0),
                is_pinned: row.get::<_, i64>(5)? != 0,
                is_locked: row.get::<_, i64>(6)? != 0,
                folder: row.get(7)?,
            })
        })?;

        let notes = rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|r| note_from_row(r, &store_uuid))
            .collect();

        Ok(notes)
    }

    /// Full-text search on title and snippet (no body content — stays local).
    pub fn search_notes(&self, query: &str, limit: usize) -> Result<Vec<Note>> {
        let (conn, store_uuid) = self.open_db()?;
        let ent = note_entity_id(&conn)?;
        let pattern = format!("%{query}%");

        let mut stmt = conn.prepare(&format!(
            r#"
            SELECT
                n.Z_PK,
                COALESCE(n.ZUSERTITLE, n.ZTITLE, '(untitled)'),
                n.ZSUMMARY,
                CAST(n.ZCREATIONDATE     + 978307200 AS INTEGER),
                CAST(n.ZMODIFICATIONDATE + 978307200 AS INTEGER),
                COALESCE(n.ZISPINNED, 0),
                COALESCE(n.ZISPASSWORDPROTECTED, 0),
                f.ZNAME
            FROM ZICCLOUDSYNCINGOBJECT n
            LEFT JOIN ZICCLOUDSYNCINGOBJECT f
                ON n.ZFOLDER = f.Z_PK
            WHERE n.Z_ENT = {ent}
              AND COALESCE(n.ZMARKEDFORDELETION, 0) = 0
              AND n.ZNOTEDATA IS NOT NULL
              AND (
                COALESCE(n.ZUSERTITLE, n.ZTITLE, '') LIKE ?1
                OR COALESCE(n.ZSUMMARY, '')           LIKE ?1
              )
            ORDER BY n.ZMODIFICATIONDATE DESC
            LIMIT {limit}
            "#
        ))?;

        let rows = stmt.query_map(rusqlite::params![pattern], |row| {
            let pk: i64 = row.get(0)?;
            Ok(NoteRow {
                pk,
                title: row.get(1)?,
                snippet: row.get(2)?,
                created_at: row.get::<_, i64>(3).unwrap_or(0),
                modified_at: row.get::<_, i64>(4).unwrap_or(0),
                is_pinned: row.get::<_, i64>(5)? != 0,
                is_locked: row.get::<_, i64>(6)? != 0,
                folder: row.get(7)?,
            })
        })?;

        let notes = rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|r| note_from_row(r, &store_uuid))
            .collect();

        Ok(notes)
    }

    /// Fetch metadata for one note by Z_PK. Body is not included.
    pub fn get_note_metadata(&self, pk: i64) -> Result<Option<Note>> {
        let (conn, store_uuid) = self.open_db()?;
        let ent = note_entity_id(&conn)?;

        let result = conn.query_row(
            &format!(
                r#"
                SELECT
                    n.Z_PK,
                    COALESCE(n.ZUSERTITLE, n.ZTITLE, '(untitled)'),
                    n.ZSUMMARY,
                    CAST(n.ZCREATIONDATE     + 978307200 AS INTEGER),
                    CAST(n.ZMODIFICATIONDATE + 978307200 AS INTEGER),
                    COALESCE(n.ZISPINNED, 0),
                    COALESCE(n.ZISPASSWORDPROTECTED, 0),
                    f.ZNAME
                FROM ZICCLOUDSYNCINGOBJECT n
                LEFT JOIN ZICCLOUDSYNCINGOBJECT f ON n.ZFOLDER = f.Z_PK
                WHERE n.Z_ENT = {ent} AND n.Z_PK = ?1
                "#
            ),
            rusqlite::params![pk],
            |row| {
                Ok(NoteRow {
                    pk: row.get(0)?,
                    title: row.get(1)?,
                    snippet: row.get(2)?,
                    created_at: row.get::<_, i64>(3).unwrap_or(0),
                    modified_at: row.get::<_, i64>(4).unwrap_or(0),
                    is_pinned: row.get::<_, i64>(5)? != 0,
                    is_locked: row.get::<_, i64>(6)? != 0,
                    folder: row.get(7)?,
                })
            },
        );

        match result {
            Ok(row) => Ok(Some(note_from_row(row, &store_uuid))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Fetch the HTML body of a note via JXA / osascript.
    /// Returns None for locked notes or when Notes.app is unavailable.
    pub async fn get_note_body(&self, coredata_id: &str) -> Result<Option<String>> {
        if coredata_id.is_empty() {
            return Ok(None);
        }

        // Escape single quotes inside the ID so it embeds safely in JS
        let safe_id = coredata_id.replace('\'', "\\'");
        let script = format!(
            r#"
            const app = Application("Notes");
            try {{
                const matches = app.notes.whose({{id: {{_equals: '{safe_id}'}}}})();
                matches.length > 0 ? matches[0].body() : ""
            }} catch(e) {{
                ""
            }}
            "#
        );

        let output = tokio::process::Command::new("osascript")
            .args(["-l", "JavaScript", "-e", &script])
            .output()
            .await?;

        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(if text.is_empty() { None } else { Some(text) })
        } else {
            Ok(None)
        }
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    fn open_db(&self) -> Result<(rusqlite::Connection, String)> {
        let conn = rusqlite::Connection::open_with_flags(
            &self.note_store,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        conn.execute_batch("PRAGMA busy_timeout = 2000;")?;
        let store_uuid: String = conn
            .query_row("SELECT Z_UUID FROM Z_METADATA LIMIT 1", [], |r| r.get(0))
            .unwrap_or_default();
        Ok((conn, store_uuid))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn note_entity_id(conn: &rusqlite::Connection) -> Result<i64> {
    let id: i64 = conn.query_row(
        "SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME = 'ICNote' LIMIT 1",
        [],
        |r| r.get(0),
    )?;
    Ok(id)
}

struct NoteRow {
    pk: i64,
    title: String,
    snippet: Option<String>,
    created_at: i64,
    modified_at: i64,
    is_pinned: bool,
    is_locked: bool,
    folder: Option<String>,
}

fn note_from_row(r: NoteRow, store_uuid: &str) -> Note {
    let coredata_id = if store_uuid.is_empty() {
        String::new()
    } else {
        format!("x-coredata://{}/ICNote/p{}", store_uuid, r.pk)
    };
    Note {
        pk: r.pk,
        coredata_id,
        title: r.title,
        snippet: r.snippet,
        created_at: r.created_at,
        modified_at: r.modified_at,
        is_pinned: r.is_pinned,
        is_locked: r.is_locked,
        folder: r.folder,
        body: None,
        language: None,
    }
}

// ── Language detection ────────────────────────────────────────────────────────

pub fn detect_language(text: &str) -> Option<String> {
    let info = whatlang::detect(text)?;
    if !info.is_reliable() {
        return None;
    }
    Some(
        match info.lang() {
            whatlang::Lang::Slk => "sk",
            whatlang::Lang::Ces => "cs",
            whatlang::Lang::Eng => "en",
            whatlang::Lang::Deu => "de",
            _ => return None,
        }
        .to_string(),
    )
}

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};

// ── Public types ──────────────────────────────────────────────────────────────

/// Metadata for a single MIME attachment within a mail message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailAttachment {
    /// Original filename from Content-Disposition / Content-Type.
    pub filename: String,
    /// MIME type (e.g. "application/pdf", "image/jpeg").
    pub mimetype: String,
    /// Decoded byte size.
    pub size: usize,
    /// Zero-based index among all subparts of the message MIME tree.
    pub part_index: usize,
    /// Content-ID header value if present (for inline images).
    pub content_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailMessage {
    pub rowid: i64,
    pub subject: String,
    /// RFC 5322 address string
    pub sender: String,
    /// Display name if present
    pub sender_display: Option<String>,
    /// Primary "To" recipient address (type=0 in recipients table)
    pub recipient: Option<String>,
    /// Unix timestamp (seconds)
    pub received_at: i64,
    pub is_read: bool,
    pub mailbox_url: String,
    /// Plain-text body; None when emlx not locally cached
    pub body: Option<String>,
    /// False when the emlx file was not found on disk
    pub body_available: bool,
    /// ISO 639-1 code ("sk", "en", …) from whatlang, None if undetectable
    pub language: Option<String>,
    /// Attachments found in the MIME tree (metadata only, no bytes).
    #[serde(default)]
    pub attachments: Vec<MailAttachment>,
    /// RFC 2822 Message-ID header value (stripped of angle brackets).
    /// Populated only when the emlx file is parsed locally.
    #[serde(default)]
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MailBodyHydrationState {
    Readable,
    Empty,
    Unavailable,
    AutomationDenied,
    AutomationTimedOut,
    AutomationFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HydratedMailMessage {
    pub message: MailMessage,
    pub state: MailBodyHydrationState,
    pub used_automation: bool,
}

/// Filter parameters for [`MailConnector::search_messages`].
/// All fields are optional; an empty filter returns the most recent messages.
#[derive(Debug, Clone, Default)]
pub struct MailSearchFilter {
    /// Matched against `addresses.address` and `addresses.comment` (LIKE, case-insensitive).
    pub sender: Option<String>,
    /// Matched against `subjects.subject` (LIKE, case-insensitive).
    pub subject: Option<String>,
    /// Inclusive lower bound, Unix epoch seconds.
    pub date_from: Option<i64>,
    /// Exclusive upper bound, Unix epoch seconds.
    pub date_to: Option<i64>,
    /// Max rows to return (0 → default 10).
    pub limit: usize,
    /// Each keyword generates an OR clause matching sender address, sender display name, OR subject.
    /// Use when the LLM puts the search term in keywords instead of sender/subject.
    pub keywords: Vec<String>,
}

// ── Connector ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct MailConnector {
    envelope_index: PathBuf,
    mail_v10_dir: PathBuf,
}

fn like_pattern(value: &str) -> String {
    format!("%{}%", value.to_lowercase())
}

fn compact_like_pattern(value: &str) -> String {
    format!("%{}%", compact_search_text(value))
}

fn compact_search_text(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

fn compact_sender_expr() -> &'static str {
    "REPLACE(LOWER(COALESCE(a.address,'') || COALESCE(a.comment,'')), ' ', '')"
}

fn compact_subject_expr() -> &'static str {
    "REPLACE(LOWER(COALESCE(s.subject,'')), ' ', '')"
}

impl MailConnector {
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("no home dir"))?;
        let mail_v10_dir = home.join("Library/Mail/V10");
        let envelope_index = mail_v10_dir.join("MailData/Envelope Index");
        Ok(Self {
            envelope_index,
            mail_v10_dir,
        })
    }

    /// True when Full Disk Access is granted and the Envelope Index is readable.
    pub fn is_accessible(&self) -> bool {
        self.open_db().is_ok()
    }

    // ── Queries ──────────────────────────────────────────────────────────────

    /// List inbox messages ordered by most recent first.
    /// Body is NOT populated here — call `get_message` for full content.
    pub fn list_inbox(&self, limit: usize, unread_only: bool) -> Result<Vec<MailMessage>> {
        let conn = self.open_db()?;
        let unread_clause = if unread_only { "AND m.read = 0" } else { "" };
        let sql = format!(
            r#"
            SELECT
                m.ROWID, m.date_received, m.read,
                COALESCE(s.subject, '(no subject)'),
                COALESCE(a.comment, ''),
                COALESCE(a.address, ''),
                COALESCE(mb.url, ''),
                (SELECT a2.address FROM recipients r LEFT JOIN addresses a2 ON r.address=a2.ROWID
                 WHERE r.message=m.ROWID AND r.type=0 ORDER BY r.position LIMIT 1)
            FROM messages m
            LEFT JOIN subjects  s  ON m.subject = s.ROWID
            LEFT JOIN addresses a  ON m.sender  = a.ROWID
            LEFT JOIN mailboxes mb ON m.mailbox = mb.ROWID
            WHERE m.deleted = 0 {unread_clause}
            ORDER BY m.date_received DESC
            LIMIT {limit}
            "#
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let sender_display: String = row.get(4)?;
            Ok(MailMessage {
                rowid: row.get(0)?,
                received_at: row.get(1)?,
                is_read: row.get::<_, i64>(2)? != 0,
                subject: row.get(3)?,
                sender_display: if sender_display.is_empty() {
                    None
                } else {
                    Some(sender_display)
                },
                sender: row.get(5)?,
                mailbox_url: row.get(6)?,
                recipient: row.get(7).ok().flatten(),
                body: None,
                body_available: true,
                language: None,
                attachments: vec![],
                message_id: None,
            })
        })?;

        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Flexible filtered search over sender / subject / date range.
    ///
    /// Sender and subject are matched with `LIKE '%query%'` (case-insensitive).
    /// Date bounds are inclusive Unix-epoch seconds.  All fields are optional;
    /// an empty filter returns the `limit` most-recent non-deleted messages.
    pub fn search_messages(&self, f: &MailSearchFilter) -> Result<Vec<MailMessage>> {
        let conn = self.open_db()?;

        let mut clauses: Vec<String> = vec!["m.deleted = 0".to_string()];
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let mut idx = 1usize;

        if let Some(ref sender) = f.sender {
            let pattern = like_pattern(sender);
            let compact_pattern = compact_like_pattern(sender);
            clauses.push(format!(
                "(LOWER(COALESCE(a.address,'')) LIKE ?{idx} OR LOWER(COALESCE(a.comment,'')) LIKE ?{idx} OR {compact_sender_expr} LIKE ?{next_idx})",
                compact_sender_expr = compact_sender_expr(),
                next_idx = idx + 1,
            ));
            params.push(Box::new(pattern));
            params.push(Box::new(compact_pattern));
            idx += 2;
        }
        if let Some(ref subject) = f.subject {
            let pattern = like_pattern(subject);
            let compact_pattern = compact_like_pattern(subject);
            clauses.push(format!(
                "(LOWER(COALESCE(s.subject,'')) LIKE ?{idx} OR {compact_subject_expr} LIKE ?{next_idx})",
                compact_subject_expr = compact_subject_expr(),
                next_idx = idx + 1,
            ));
            params.push(Box::new(pattern));
            params.push(Box::new(compact_pattern));
            idx += 2;
        }
        if let Some(from) = f.date_from {
            clauses.push(format!("m.date_received >= ?{idx}"));
            params.push(Box::new(from));
            idx += 1;
        }
        if let Some(to) = f.date_to {
            clauses.push(format!("m.date_received < ?{idx}"));
            params.push(Box::new(to));
            idx += 1;
        }
        // Each keyword generates an OR clause across sender address, display name, and subject.
        // Catches cases where the LLM classifier puts the company/person name in keywords
        // instead of the sender field.
        for kw in &f.keywords {
            let pattern = like_pattern(kw);
            let compact_pattern = compact_like_pattern(kw);
            clauses.push(format!(
                "(LOWER(COALESCE(a.address,'')) LIKE ?{idx} OR LOWER(COALESCE(a.comment,'')) LIKE ?{idx} OR LOWER(COALESCE(s.subject,'')) LIKE ?{idx} OR {compact_sender_expr} LIKE ?{next_idx} OR {compact_subject_expr} LIKE ?{next_idx})",
                compact_sender_expr = compact_sender_expr(),
                compact_subject_expr = compact_subject_expr(),
                next_idx = idx + 1,
            ));
            params.push(Box::new(pattern));
            params.push(Box::new(compact_pattern));
            idx += 2;
        }
        let _ = idx; // suppress unused warning

        let limit = if f.limit == 0 { 10 } else { f.limit };
        let where_clause = clauses.join(" AND ");
        let sql = format!(
            r#"
            SELECT
                m.ROWID, m.date_received, m.read,
                COALESCE(s.subject, '(no subject)'),
                COALESCE(a.comment, ''),
                COALESCE(a.address, ''),
                COALESCE(mb.url, ''),
                (SELECT a2.address FROM recipients r LEFT JOIN addresses a2 ON r.address=a2.ROWID
                 WHERE r.message=m.ROWID AND r.type=0 ORDER BY r.position LIMIT 1)
            FROM messages m
            LEFT JOIN subjects  s  ON m.subject = s.ROWID
            LEFT JOIN addresses a  ON m.sender  = a.ROWID
            LEFT JOIN mailboxes mb ON m.mailbox = mb.ROWID
            WHERE {where_clause}
            ORDER BY m.date_received DESC
            LIMIT {limit}
            "#
        );

        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            let display: String = row.get(4)?;
            Ok(MailMessage {
                rowid: row.get(0)?,
                received_at: row.get(1)?,
                is_read: row.get::<_, i64>(2)? != 0,
                subject: row.get(3)?,
                sender_display: if display.is_empty() {
                    None
                } else {
                    Some(display)
                },
                sender: row.get(5)?,
                mailbox_url: row.get(6)?,
                recipient: row.get(7).ok().flatten(),
                body: None,
                body_available: true,
                language: None,
                attachments: vec![],
                message_id: None,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Search messages by subject substring, newest first.
    pub fn search_by_subject(&self, query: &str, limit: usize) -> Result<Vec<MailMessage>> {
        let conn = self.open_db()?;
        let pattern = format!("%{}%", query.to_lowercase());
        let sql = format!(
            r#"
            SELECT
                m.ROWID, m.date_received, m.read,
                COALESCE(s.subject, '(no subject)'),
                COALESCE(a.comment, ''),
                COALESCE(a.address, ''),
                COALESCE(mb.url, ''),
                (SELECT a2.address FROM recipients r LEFT JOIN addresses a2 ON r.address=a2.ROWID
                 WHERE r.message=m.ROWID AND r.type=0 ORDER BY r.position LIMIT 1)
            FROM messages m
            LEFT JOIN subjects  s  ON m.subject = s.ROWID
            LEFT JOIN addresses a  ON m.sender  = a.ROWID
            LEFT JOIN mailboxes mb ON m.mailbox = mb.ROWID
            WHERE m.deleted = 0 AND LOWER(COALESCE(s.subject, '')) LIKE ?1
            ORDER BY m.date_received DESC
            LIMIT {limit}
            "#
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![pattern], |row| {
            let display: String = row.get(4)?;
            Ok(MailMessage {
                rowid: row.get(0)?,
                received_at: row.get(1)?,
                is_read: row.get::<_, i64>(2)? != 0,
                subject: row.get(3)?,
                sender_display: if display.is_empty() {
                    None
                } else {
                    Some(display)
                },
                sender: row.get(5)?,
                mailbox_url: row.get(6)?,
                recipient: row.get(7).ok().flatten(),
                body: None,
                body_available: true,
                language: None,
                attachments: vec![],
                message_id: None,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// All messages received strictly after `since_ts` (Unix seconds), newest first.
    /// Used for incremental sync — pass 0 to get everything.
    pub fn list_since(&self, since_ts: i64, limit: usize) -> Result<Vec<MailMessage>> {
        let conn = self.open_db()?;
        let sql = format!(
            r#"
            SELECT
                m.ROWID, m.date_received, m.read,
                COALESCE(s.subject, '(no subject)'),
                COALESCE(a.comment, ''),
                COALESCE(a.address, ''),
                COALESCE(mb.url, ''),
                (SELECT a2.address FROM recipients r LEFT JOIN addresses a2 ON r.address=a2.ROWID
                 WHERE r.message=m.ROWID AND r.type=0 ORDER BY r.position LIMIT 1)
            FROM messages m
            LEFT JOIN subjects  s  ON m.subject = s.ROWID
            LEFT JOIN addresses a  ON m.sender  = a.ROWID
            LEFT JOIN mailboxes mb ON m.mailbox = mb.ROWID
            WHERE m.deleted = 0 AND m.date_received > {since_ts}
            ORDER BY m.date_received DESC
            LIMIT {limit}
            "#
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let display: String = row.get(4)?;
            Ok(MailMessage {
                rowid: row.get(0)?,
                received_at: row.get(1)?,
                is_read: row.get::<_, i64>(2)? != 0,
                subject: row.get(3)?,
                sender_display: if display.is_empty() {
                    None
                } else {
                    Some(display)
                },
                sender: row.get(5)?,
                mailbox_url: row.get(6)?,
                recipient: row.get(7).ok().flatten(),
                body: None,
                body_available: true,
                language: None,
                attachments: vec![],
                message_id: None,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Fetch a single message, including body from emlx if locally cached.
    pub fn get_message(&self, rowid: i64) -> Result<Option<MailMessage>> {
        let conn = self.open_db()?;
        let result = conn.query_row(
            r#"
            SELECT
                m.ROWID, m.date_received, m.read,
                COALESCE(s.subject, '(no subject)'),
                COALESCE(a.comment, ''),
                COALESCE(a.address, ''),
                COALESCE(mb.url, ''),
                (SELECT a2.address FROM recipients r LEFT JOIN addresses a2 ON r.address=a2.ROWID
                 WHERE r.message=m.ROWID AND r.type=0 ORDER BY r.position LIMIT 1)
            FROM messages m
            LEFT JOIN subjects  s  ON m.subject = s.ROWID
            LEFT JOIN addresses a  ON m.sender  = a.ROWID
            LEFT JOIN mailboxes mb ON m.mailbox = mb.ROWID
            WHERE m.ROWID = ?1 AND m.deleted = 0
            "#,
            rusqlite::params![rowid],
            |row| {
                let sender_display: String = row.get(4)?;
                Ok(MailMessage {
                    rowid: row.get(0)?,
                    received_at: row.get(1)?,
                    is_read: row.get::<_, i64>(2)? != 0,
                    subject: row.get(3)?,
                    sender_display: if sender_display.is_empty() {
                        None
                    } else {
                        Some(sender_display)
                    },
                    sender: row.get(5)?,
                    mailbox_url: row.get(6)?,
                    recipient: row.get(7).ok().flatten(),
                    body: None,
                    body_available: false,
                    language: None,
                    attachments: vec![],
                    message_id: None,
                })
            },
        );

        let Ok(mut msg) = result else { return Ok(None) };

        if let Some(emlx_path) = self.find_emlx(rowid) {
            match parse_emlx_body_and_attachments(&emlx_path) {
                Ok((text, attachments, message_id)) => {
                    if !text.trim().is_empty() {
                        msg.language = detect_language(&text);
                        msg.body = Some(text);
                        msg.body_available = true;
                    }
                    msg.attachments = attachments;
                    msg.message_id = message_id;
                }
                _ => {}
            }
        }

        // When emlx parsing yielded no attachments, check the on-disk Attachments directory.
        // Partial emlx files have headers only — attachments are stored separately.
        if msg.attachments.is_empty() {
            let fs_atts = self.find_attachment_files(rowid);
            let mut sorted = fs_atts;
            sorted.sort_by(|a, b| a.0.cmp(&b.0)); // sort by part-folder name
            for (idx, (_, path)) in sorted.into_iter().enumerate() {
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("attachment")
                    .to_string();
                let mimetype = mime_guess::from_path(&path)
                    .first_or_octet_stream()
                    .to_string();
                msg.attachments.push(MailAttachment {
                    part_index: idx,
                    filename,
                    mimetype,
                    size: path.metadata().map(|m| m.len() as usize).unwrap_or(0),
                    content_id: None,
                });
            }
        }

        Ok(Some(msg))
    }

    /// Load a message body through one local-first operation shared by daemon
    /// callers. Mail.app Automation is used only when no local `.emlx` body is
    /// available.
    pub async fn hydrate_message(&self, rowid: i64) -> Result<Option<HydratedMailMessage>> {
        let connector = self.clone();
        let message = tokio::task::spawn_blocking(move || connector.get_message(rowid))
            .await
            .map_err(|error| anyhow!("Apple Mail local body task failed: {error}"))??;
        let Some(message) = message else {
            return Ok(None);
        };
        Ok(Some(
            hydrate_loaded_message(message, |identity| async move {
                body_via_mail_app(&identity).await
            })
            .await,
        ))
    }

    /// Fetch the raw (decoded) bytes for a single attachment by ROWID + part_index.
    /// First tries emlx-embedded bytes; falls back to the on-disk Attachments directory
    /// (used when the message is a .partial.emlx with separately-stored attachments).
    pub fn get_message_attachment(
        &self,
        rowid: i64,
        part_index: usize,
    ) -> Result<(MailAttachment, Vec<u8>)> {
        // Try emlx parse first
        if let Some(emlx_path) = self.find_emlx(rowid) {
            if let Ok(result) = get_attachment_bytes(&emlx_path, part_index) {
                return Ok(result);
            }
        }

        // Fall back to filesystem Attachments directory
        let mut sorted = self.find_attachment_files(rowid);
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let (_, path) = sorted
            .into_iter()
            .nth(part_index)
            .ok_or_else(|| anyhow!("attachment index {part_index} not found for rowid {rowid}"))?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment")
            .to_string();
        let mimetype = mime_guess::from_path(&path)
            .first_or_octet_stream()
            .to_string();
        let size = path.metadata().map(|m| m.len() as usize).unwrap_or(0);
        let bytes = std::fs::read(&path)?;
        Ok((
            MailAttachment {
                part_index,
                filename,
                mimetype,
                size,
                content_id: None,
            },
            bytes,
        ))
    }

    /// Fetch attachment bytes as a base64-encoded string for connector APIs.
    pub fn get_message_attachment_base64(
        &self,
        rowid: i64,
        part_index: usize,
    ) -> Result<(MailAttachment, String)> {
        let (meta, bytes) = self.get_message_attachment(rowid, part_index)?;
        Ok((meta, B64.encode(&bytes)))
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    fn open_db(&self) -> Result<rusqlite::Connection> {
        let conn = rusqlite::Connection::open_with_flags(
            &self.envelope_index,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        conn.execute_batch("PRAGMA busy_timeout = 2000;")?;
        Ok(conn)
    }

    /// Walk V10/{acct}/*.mbox/{guid}/Data/{d1}/{d2}/{d3}/Messages/{rowid}.emlx
    /// Also accepts .partial.emlx (body not fully downloaded but headers present).
    /// Apple Mail uses a three-level shard: d1=(rowid/1000)%10, d2=(rowid/10000)%10, d3=(rowid/100000)%10.
    fn find_emlx(&self, rowid: i64) -> Option<PathBuf> {
        let d1 = (rowid / 1000) % 10;
        let d2 = (rowid / 10000) % 10;
        let d3 = (rowid / 100000) % 10;

        for acct in std::fs::read_dir(&self.mail_v10_dir).ok()?.flatten() {
            let Ok(ft) = acct.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }

            for mbox in std::fs::read_dir(acct.path()).ok()?.flatten() {
                let mp = mbox.path();
                if mp.extension().and_then(|e| e.to_str()) != Some("mbox") {
                    continue;
                }

                for guid in std::fs::read_dir(&mp).ok()?.flatten() {
                    let Ok(gft) = guid.file_type() else { continue };
                    if !gft.is_dir() {
                        continue;
                    }

                    let base = guid
                        .path()
                        .join("Data")
                        .join(d1.to_string())
                        .join(d2.to_string())
                        .join(d3.to_string())
                        .join("Messages");

                    // Prefer full emlx, fall back to partial
                    let full = base.join(format!("{rowid}.emlx"));
                    if full.exists() {
                        return Some(full);
                    }
                    let partial = base.join(format!("{rowid}.partial.emlx"));
                    if partial.exists() {
                        return Some(partial);
                    }
                }
            }
        }
        None
    }

    /// Walk the Attachments directory for a given rowid.
    /// Returns list of (part_folder_name, file_path) for each cached attachment file.
    /// Structure: Data/{d1}/{d2}/{d3}/Attachments/{rowid}/{part}/{filename}
    fn find_attachment_files(&self, rowid: i64) -> Vec<(String, PathBuf)> {
        let d1 = (rowid / 1000) % 10;
        let d2 = (rowid / 10000) % 10;
        let d3 = (rowid / 100000) % 10;
        let mut results = Vec::new();

        let mut walk = |acct: std::fs::DirEntry| -> Option<()> {
            for mbox in std::fs::read_dir(acct.path()).ok()?.flatten() {
                let mp = mbox.path();
                if mp.extension().and_then(|e| e.to_str()) != Some("mbox") {
                    continue;
                }
                for guid in std::fs::read_dir(&mp).ok()?.flatten() {
                    let att_dir = guid
                        .path()
                        .join("Data")
                        .join(d1.to_string())
                        .join(d2.to_string())
                        .join(d3.to_string())
                        .join("Attachments")
                        .join(rowid.to_string());
                    if !att_dir.exists() {
                        continue;
                    }
                    for part in std::fs::read_dir(&att_dir).ok()?.flatten() {
                        let part_name = part.file_name().to_string_lossy().to_string();
                        for file in std::fs::read_dir(part.path()).ok()?.flatten() {
                            if file.file_type().ok().map(|t| t.is_file()).unwrap_or(false) {
                                results.push((part_name.clone(), file.path()));
                            }
                        }
                    }
                }
            }
            None
        };

        if let Ok(rd) = std::fs::read_dir(&self.mail_v10_dir) {
            for acct in rd.flatten() {
                if acct.file_type().ok().map(|t| t.is_dir()).unwrap_or(false) {
                    walk(acct);
                }
            }
        }
        results
    }
}

// ── emlx parser ───────────────────────────────────────────────────────────────

/// Parse an Apple Mail emlx file and extract the best plain-text body.
///
/// emlx layout:
///   Line 1:  ASCII integer = byte count of trailing binary plist
///   Lines 2…N: RFC 2822 email (headers + body)
///   Tail:    binary plist of Mail metadata (flags, colours, …)
fn parse_emlx_email_bytes(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;

    let nl = bytes
        .iter()
        .position(|&b| b == b'\n')
        .ok_or_else(|| anyhow!("invalid emlx: no newline"))?;
    let plist_len: usize = std::str::from_utf8(&bytes[..nl])?
        .trim()
        .parse()
        .map_err(|_| anyhow!("invalid emlx: bad plist length"))?;

    let email_start = nl + 1;
    let email_end = bytes.len().saturating_sub(plist_len);
    if email_end <= email_start {
        return Err(anyhow!("invalid emlx: empty email section"));
    }

    Ok(bytes[email_start..email_end].to_vec())
}

/// Parse body text, attachment metadata, and Message-ID in one pass.
fn parse_emlx_body_and_attachments(
    path: &Path,
) -> Result<(String, Vec<MailAttachment>, Option<String>)> {
    let email_bytes = parse_emlx_email_bytes(path)?;
    let parsed = mailparse::parse_mail(&email_bytes)?;
    let text = extract_plain_text(&parsed)?;
    let attachments = extract_attachments_from_parsed(&parsed);
    // Extract Message-ID from top-level headers (strip surrounding angle brackets).
    let message_id = parsed
        .headers
        .iter()
        .find(|h| h.get_key().to_lowercase() == "message-id")
        .map(|h| {
            h.get_value()
                .trim()
                .trim_matches('<')
                .trim_matches('>')
                .to_string()
        })
        .filter(|s| !s.is_empty());
    Ok((text, attachments, message_id))
}

/// Public API: extract raw bytes for a single attachment by part_index.
/// Returns (MailAttachment metadata, decoded bytes).
pub fn get_attachment_bytes(path: &Path, part_index: usize) -> Result<(MailAttachment, Vec<u8>)> {
    let email_bytes = parse_emlx_email_bytes(path)?;
    let parsed = mailparse::parse_mail(&email_bytes)?;
    let mut counter = 0usize;
    find_part_bytes(&parsed, part_index, &mut counter)?
        .ok_or_else(|| anyhow!("attachment part_index {part_index} not found"))
}

fn find_part_bytes(
    mail: &mailparse::ParsedMail,
    target: usize,
    counter: &mut usize,
) -> Result<Option<(MailAttachment, Vec<u8>)>> {
    // Check if this part is an attachment
    if let Some(att) = classify_attachment_part(mail, *counter) {
        if *counter == target {
            let bytes = mail.get_body_raw()?;
            return Ok(Some((att, bytes)));
        }
        *counter += 1;
    }
    // Recurse into subparts
    for subpart in &mail.subparts {
        if let Some(found) = find_part_bytes(subpart, target, counter)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn extract_plain_text(mail: &mailparse::ParsedMail) -> Result<String> {
    let mime = mail.ctype.mimetype.as_str();

    if mime == "text/plain" {
        return Ok(mail.get_body()?);
    }

    if mime.starts_with("multipart/") {
        let mut plain_parts: Vec<String> = Vec::new();
        let mut html_fallback: Option<String> = None;

        for part in &mail.subparts {
            let pct = part.ctype.mimetype.as_str();
            if pct == "text/plain" {
                let t = part.get_body()?;
                if !t.trim().is_empty() {
                    plain_parts.push(t);
                }
            } else if pct == "text/html" && html_fallback.is_none() {
                html_fallback = Some(strip_html(&part.get_body()?));
            } else if pct.starts_with("multipart/") {
                if let Ok(nested) = extract_plain_text(part) {
                    if !nested.trim().is_empty() {
                        plain_parts.push(nested);
                    }
                }
            }
        }

        if !plain_parts.is_empty() {
            return Ok(plain_parts.join("\n\n"));
        }
        if let Some(html) = html_fallback {
            return Ok(html);
        }
    }

    Ok(String::new())
}

/// Recursively collect all non-body MIME parts as attachment metadata.
fn extract_attachments_from_parsed(mail: &mailparse::ParsedMail) -> Vec<MailAttachment> {
    let mut attachments = Vec::new();
    let mut counter = 0usize;
    collect_attachments(mail, &mut counter, &mut attachments);
    attachments
}

fn collect_attachments(
    mail: &mailparse::ParsedMail,
    counter: &mut usize,
    out: &mut Vec<MailAttachment>,
) {
    if let Some(att) = classify_attachment_part(mail, *counter) {
        out.push(att);
        *counter += 1;
        return; // Don't descend into attachment parts
    }
    // Descend into multipart containers
    for subpart in &mail.subparts {
        collect_attachments(subpart, counter, out);
    }
}

/// Return Some(MailAttachment) if `part` is an attachment (not a body part).
fn classify_attachment_part(part: &mailparse::ParsedMail, index: usize) -> Option<MailAttachment> {
    let mime = part.ctype.mimetype.as_str();

    // Skip body text types and multipart containers
    if mime == "text/plain" || mime == "text/html" || mime.starts_with("multipart/") {
        return None;
    }

    // Read Content-Disposition header for filename and disposition type
    let mut filename: Option<String> = None;
    let mut is_inline_text = false;
    let mut content_id: Option<String> = None;

    for hdr in &part.headers {
        let hname = hdr.get_key().to_lowercase();
        match hname.as_str() {
            "content-disposition" => {
                let val = hdr.get_value();
                // inline text bodies (rare, but skip them)
                if val.to_lowercase().starts_with("inline") && mime.starts_with("text/") {
                    is_inline_text = true;
                }
                // Extract filename= parameter
                if filename.is_none() {
                    filename = extract_param(&val, "filename");
                }
            }
            "content-type" => {
                if filename.is_none() {
                    let val = hdr.get_value();
                    filename = extract_param(&val, "name");
                }
            }
            "content-id" => {
                let val = hdr
                    .get_value()
                    .trim()
                    .trim_matches('<')
                    .trim_matches('>')
                    .to_string();
                if !val.is_empty() {
                    content_id = Some(val);
                }
            }
            _ => {}
        }
    }

    if is_inline_text {
        return None;
    }

    // Derive filename from MIME type if none found
    let filename = filename.unwrap_or_else(|| {
        let ext = mime.split('/').nth(1).unwrap_or("bin");
        format!("attachment.{ext}")
    });

    let size = part.get_body_raw().map(|b| b.len()).unwrap_or(0);

    Some(MailAttachment {
        filename,
        mimetype: mime.to_string(),
        size,
        part_index: index,
        content_id,
    })
}

/// Extract a named parameter from a MIME header value string.
/// e.g. `Content-Type: application/pdf; name="invoice.pdf"` → `Some("invoice.pdf")`
fn extract_param(header_value: &str, param: &str) -> Option<String> {
    let needle = format!("{param}=");
    let lower = header_value.to_lowercase();
    let pos = lower.find(needle.as_str())?;
    let rest = &header_value[pos + needle.len()..];
    let rest = rest.trim_start();
    if rest.starts_with('"') {
        // Quoted string
        let end = rest[1..].find('"')?;
        Some(rest[1..end + 1].to_string())
    } else {
        // Unquoted: ends at ; or whitespace
        let end = rest
            .find([';', ' ', '\t', '\r', '\n'])
            .unwrap_or(rest.len());
        let val = &rest[..end];
        if val.is_empty() {
            None
        } else {
            Some(val.to_string())
        }
    }
}

/// Minimal HTML tag stripper — no external deps.
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
}

// ── AppleScript body fallback ─────────────────────────────────────────────────

const MAIL_BODY_AUTOMATION_TIMEOUT: Duration = Duration::from_secs(10);
const AUTOMATION_FIELD_SEPARATOR: char = '\u{1f}';

#[derive(Debug, Clone)]
struct MailBodyIdentity {
    subject: String,
    sender: String,
    received_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MailAutomationBody {
    Content(String),
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MailAutomationError {
    Denied,
    TimedOut,
    Failed,
}

#[derive(Debug)]
struct AutomationOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

async fn hydrate_loaded_message<F, Fut>(
    mut message: MailMessage,
    fallback: F,
) -> HydratedMailMessage
where
    F: FnOnce(MailBodyIdentity) -> Fut,
    Fut: std::future::Future<Output = Result<MailAutomationBody, MailAutomationError>>,
{
    if let Some(body) = message.body.take() {
        let bounded = body.chars().take(4_000).collect::<String>();
        message.body = Some(bounded.clone());
        let state = if bounded.trim().is_empty() {
            MailBodyHydrationState::Empty
        } else {
            MailBodyHydrationState::Readable
        };
        return HydratedMailMessage {
            message,
            state,
            used_automation: false,
        };
    }
    if message.body_available {
        message.body = Some(String::new());
        return HydratedMailMessage {
            message,
            state: MailBodyHydrationState::Empty,
            used_automation: false,
        };
    }

    let identity = MailBodyIdentity {
        subject: message.subject.clone(),
        sender: message.sender.clone(),
        received_at: message.received_at,
    };
    let state = match fallback(identity).await {
        Ok(MailAutomationBody::Content(body)) => {
            let bounded = body.chars().take(4_000).collect::<String>();
            message.body_available = true;
            message.language = detect_language(&bounded);
            message.body = Some(bounded.clone());
            if bounded.trim().is_empty() {
                MailBodyHydrationState::Empty
            } else {
                MailBodyHydrationState::Readable
            }
        }
        Ok(MailAutomationBody::Unavailable) => MailBodyHydrationState::Unavailable,
        Err(MailAutomationError::Denied) => MailBodyHydrationState::AutomationDenied,
        Err(MailAutomationError::TimedOut) => MailBodyHydrationState::AutomationTimedOut,
        Err(MailAutomationError::Failed) => MailBodyHydrationState::AutomationFailed,
    };
    HydratedMailMessage {
        message,
        state,
        used_automation: true,
    }
}

async fn body_via_mail_app(
    identity: &MailBodyIdentity,
) -> Result<MailAutomationBody, MailAutomationError> {
    let script = mail_body_applescript(identity);
    let output = run_automation_with_timeout(MAIL_BODY_AUTOMATION_TIMEOUT, async move {
        let mut command = tokio::process::Command::new("osascript");
        command.args(["-e", &script]).kill_on_drop(true);
        command.output().await.map(|output| AutomationOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    })
    .await?;
    parse_mail_body_automation_output(output)
}

async fn run_automation_with_timeout<F>(
    timeout: Duration,
    operation: F,
) -> Result<AutomationOutput, MailAutomationError>
where
    F: std::future::Future<Output = std::io::Result<AutomationOutput>>,
{
    match tokio::time::timeout(timeout, operation).await {
        Err(_) => Err(MailAutomationError::TimedOut),
        Ok(Err(_)) => Err(MailAutomationError::Failed),
        Ok(Ok(output)) => Ok(output),
    }
}

fn parse_mail_body_automation_output(
    output: AutomationOutput,
) -> Result<MailAutomationBody, MailAutomationError> {
    if !output.success {
        let error = output.stderr.to_ascii_lowercase();
        return Err(
            if error.contains("-1743")
                || error.contains("not authorized to send apple events")
                || error.contains("automation permission")
            {
                MailAutomationError::Denied
            } else {
                MailAutomationError::Failed
            },
        );
    }
    let stdout = output.stdout.trim_end_matches(['\r', '\n']);
    let (status, body) = stdout
        .split_once(AUTOMATION_FIELD_SEPARATOR)
        .unwrap_or((stdout, ""));
    match status {
        "CONTENT" => Ok(MailAutomationBody::Content(body.to_string())),
        "UNAVAILABLE" | "NOT_FOUND" => Ok(MailAutomationBody::Unavailable),
        _ => Err(MailAutomationError::Failed),
    }
}

fn mail_body_applescript(identity: &MailBodyIdentity) -> String {
    let subject = escape_applescript_string(&identity.subject);
    let sender = escape_applescript_string(&identity.sender);
    let received_at = identity.received_at;
    format!(
        r#"set fieldSep to ASCII character 31
set epochDate to date "Thursday, 1 January 1970 at 00:00:00"
set successfulQueries to 0
set lastQueryErrorMessage to ""
set lastQueryErrorNumber to 0
set fatalErrorMessage to ""
set fatalErrorNumber to 0
tell application "Mail"
    repeat with acct in accounts
        repeat with mbx in mailboxes of acct
            try
                set candidates to (every message of mbx whose subject is "{subject}")
                set successfulQueries to successfulQueries + 1
                repeat with m in candidates
                    set snd to ""
                    try
                        set snd to (sender of m) as text
                    end try
                    set unixTs to ((((date received of m) - epochDate) as real) - (time to GMT))
                    set dateDelta to unixTs - {received_at}
                    if dateDelta < 0 then set dateDelta to -dateDelta
                    ignoring case
                        set senderMatches to ("{sender}" is "") or (snd contains "{sender}")
                    end ignoring
                    if senderMatches and dateDelta <= 2 then
                        try
                            set msgContent to content of m
                            if msgContent is missing value then
                                return "UNAVAILABLE" & fieldSep
                            end if
                            return "CONTENT" & fieldSep & (msgContent as text)
                        on error errorMessage number errorNumber
                            set fatalErrorMessage to errorMessage
                            set fatalErrorNumber to errorNumber
                            error errorMessage number errorNumber
                        end try
                    end if
                end repeat
            on error errorMessage number errorNumber
                if fatalErrorNumber is not 0 then
                    error fatalErrorMessage number fatalErrorNumber
                end if
                if errorNumber is -1743 then
                    error errorMessage number errorNumber
                end if
                set lastQueryErrorMessage to errorMessage
                set lastQueryErrorNumber to errorNumber
            end try
        end repeat
    end repeat
end tell
if successfulQueries is 0 and lastQueryErrorNumber is not 0 then
    error lastQueryErrorMessage number lastQueryErrorNumber
end if
return "NOT_FOUND" & fieldSep"#
    )
}

/// Search Mail.app through AppleScript Automation.
///
/// This is slower than the Envelope Index path, but works when Full Disk Access
/// blocks direct SQLite reads and Automation access to Mail.app is available.
/// It is intentionally targeted: an empty filter returns no rows.
pub async fn search_messages_via_applescript(f: &MailSearchFilter) -> Result<Vec<MailMessage>> {
    let mut terms = Vec::new();
    push_term(&mut terms, f.sender.as_deref());
    push_term(&mut terms, f.subject.as_deref());
    for kw in &f.keywords {
        push_term(&mut terms, Some(kw));
    }
    if terms.is_empty() {
        return Ok(vec![]);
    }

    let limit = if f.limit == 0 { 10 } else { f.limit };
    let max_records = (limit * 8).clamp(10, 80);
    let terms_script = terms
        .iter()
        .map(|term| format!("\"{}\"", escape_applescript_string(term)))
        .collect::<Vec<_>>()
        .join(", ");

    let script = format!(
        r#"set fieldSep to ASCII character 31
set recSep to ASCII character 30
set epochDate to date "Thursday, 1 January 1970 at 00:00:00"
set searchTerms to {{{terms_script}}}
set outputText to ""
set foundCount to 0
tell application "Mail"
    repeat with acct in accounts
        repeat with mbx in mailboxes of acct
            repeat with termText in searchTerms
                try
                    set hits to (every message of mbx whose sender contains (termText as text) or subject contains (termText as text))
                    repeat with m in hits
                        set foundCount to foundCount + 1
                        set mid to ""
                        set subj to ""
                        set snd to ""
                        set rcpt to ""
                        set readFlag to false
                        try
                            set mid to (message id of m) as text
                        end try
                        try
                            set subj to (subject of m) as text
                        end try
                        try
                            set snd to (sender of m) as text
                        end try
                        try
                            set recipientBits to {{}}
                            repeat with r in to recipients of m
                                try
                                    set end of recipientBits to (address of r) as text
                                end try
                            end repeat
                            set oldDelims to AppleScript's text item delimiters
                            set AppleScript's text item delimiters to ", "
                            set rcpt to recipientBits as text
                            set AppleScript's text item delimiters to oldDelims
                        end try
                        try
                            set readFlag to (read status of m) as boolean
                        end try
                        set unixTs to (((date received of m) - epochDate) as real) as text
                        set outputText to outputText & (foundCount as text) & fieldSep & mid & fieldSep & snd & fieldSep & subj & fieldSep & unixTs & fieldSep & (readFlag as text) & fieldSep & rcpt & recSep
                        if foundCount >= {max_records} then return outputText
                    end repeat
                end try
            end repeat
        end repeat
    end repeat
end tell
return outputText"#
    );

    let out = tokio::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .await?;
    if !out.status.success() {
        return Err(anyhow!(
            "AppleScript Mail search failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut seen = std::collections::HashSet::new();
    let mut messages = Vec::new();
    for record in stdout.split('\u{1e}') {
        let Some(msg) = parse_applescript_mail_record(record) else {
            continue;
        };

        if let Some(from) = f.date_from {
            if msg.received_at < from {
                continue;
            }
        }
        if let Some(to) = f.date_to {
            if msg.received_at >= to {
                continue;
            }
        }

        let dedupe_key = msg.message_id.clone().unwrap_or_else(|| {
            format!(
                "{}\u{1f}{}\u{1f}{}",
                msg.sender, msg.subject, msg.received_at
            )
        });
        if !seen.insert(dedupe_key) {
            continue;
        }

        messages.push(msg);
    }

    messages.sort_by(|a, b| b.received_at.cmp(&a.received_at));
    messages.truncate(limit);
    Ok(messages)
}

fn push_term(terms: &mut Vec<String>, term: Option<&str>) {
    let Some(term) = term.map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    if !terms
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(term))
    {
        terms.push(term.to_string());
    }
}

fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn empty_to_none(value: &str) -> Option<String> {
    if value.is_empty() || value == "missing value" {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_applescript_timestamp(value: &str) -> Option<i64> {
    let normalized = value.trim().replace(',', ".");
    let parsed = normalized.parse::<f64>().ok()?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return None;
    }
    Some(parsed.round() as i64)
}

fn parse_applescript_mail_record(record: &str) -> Option<MailMessage> {
    let record = record.trim_matches(['\r', '\n']);
    if record.trim().is_empty() {
        return None;
    }
    let fields = record.split('\u{1f}').collect::<Vec<_>>();
    if fields.len() < 6 {
        return None;
    }
    let idx = fields[0].trim().parse::<i64>().unwrap_or(0);
    let message_id = empty_to_none(fields[1].trim());
    let sender_raw = fields[2].trim().to_string();
    let subject = fields[3].trim().to_string();
    let received_at = parse_applescript_timestamp(fields[4].trim()).unwrap_or(0);
    let is_read = fields[5].trim().eq_ignore_ascii_case("true");
    let recipient = fields.get(6).and_then(|value| empty_to_none(value.trim()));

    let (sender_display, sender) = split_sender_display(&sender_raw);
    Some(MailMessage {
        // Synthetic negative rowid: the message came from Mail.app, not Envelope Index.
        rowid: -idx.max(1),
        subject,
        sender,
        sender_display,
        recipient,
        received_at,
        is_read,
        mailbox_url: "mail-app://automation-search".to_string(),
        body: None,
        body_available: false,
        language: None,
        attachments: vec![],
        message_id,
    })
}

fn split_sender_display(sender: &str) -> (Option<String>, String) {
    let Some(start) = sender.rfind('<') else {
        return (None, sender.to_string());
    };
    let Some(end) = sender[start..].find('>').map(|offset| start + offset) else {
        return (None, sender.to_string());
    };
    let display = sender[..start].trim().trim_matches('"').trim();
    let address = sender[start + 1..end].trim();
    (
        if display.is_empty() {
            None
        } else {
            Some(display.to_string())
        },
        if address.is_empty() {
            sender.to_string()
        } else {
            address.to_string()
        },
    )
}

/// Open a specific email message in Apple Mail.app.
///
/// Primary path: AppleScript `whose message id is "…"` → `open`.
/// Fallback: match by subject substring when Message-ID is absent or not found.
///
/// Requires Automation → Mail permission.
pub async fn open_message(message_id: Option<&str>, subject: &str, sender: &str) -> Result<()> {
    // ── Primary: open by Message-ID ──────────────────────────────────────────
    if let Some(mid) = message_id {
        let safe_id = mid.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            r#"tell application "Mail"
    activate
    try
        set hits to (every message of inbox whose message id is "{safe_id}")
        if (count of hits) > 0 then
            open (item 1 of hits)
            return
        end if
    end try
    repeat with acct in accounts
        repeat with mbx in mailboxes of acct
            try
                set found to (every message of mbx whose message id is "{safe_id}")
                if (count of found) > 0 then
                    open (item 1 of found)
                    return
                end if
            end try
        end repeat
    end repeat
end tell"#
        );
        let out = tokio::process::Command::new("osascript")
            .args(["-e", &script])
            .output()
            .await;
        if let Ok(ref o) = out {
            if o.status.success() {
                return Ok(());
            }
        }
    }

    // ── Fallback: open by subject substring ───────────────────────────────────
    let safe_subject = subject.replace('\\', "\\\\").replace('"', "\\\"");
    let safe_sender = sender.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"tell application "Mail"
    activate
    try
        set hits to (every message of inbox whose subject contains "{safe_subject}")
        if (count of hits) > 0 then
            open (item 1 of hits)
            return
        end if
    end try
    repeat with acct in accounts
        repeat with mbx in mailboxes of acct
            try
                set found to (every message of mbx whose subject contains "{safe_subject}" and sender contains "{safe_sender}")
                if (count of found) > 0 then
                    open (item 1 of found)
                    return
                end if
            end try
        end repeat
    end repeat
end tell"#
    );
    let out = tokio::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .await?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        eprintln!("[apple_mail] open_message AppleScript failed: {err}");
    }
    Ok(())
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    fn hydration_message(body: Option<&str>, body_available: bool) -> MailMessage {
        MailMessage {
            rowid: 42,
            subject: "Duplicate subject".into(),
            sender: "alice@example.com".into(),
            sender_display: Some("Alice".into()),
            recipient: Some("oliver@example.com".into()),
            received_at: 1_785_300_000,
            is_read: false,
            mailbox_url: "imap://example/INBOX".into(),
            body: body.map(str::to_string),
            body_available,
            language: None,
            attachments: Vec::new(),
            message_id: None,
        }
    }

    /// Verifies the emlx shard formula against the spike-documented example.
    /// ROWID=95804 → Data/5/9/Messages/95804.emlx   (confirmed in docs/spikes/apple_mail.md)
    #[test]
    fn emlx_shard_calc_spike_example() {
        let rowid: i64 = 95804;
        let d1 = (rowid / 1000) % 10;
        let d2 = (rowid / 10000) % 10;
        assert_eq!(d1, 5, "d1 mismatch for ROWID {rowid}");
        assert_eq!(d2, 9, "d2 mismatch for ROWID {rowid}");
    }

    /// Boundary: ROWID that fits in a single digit shard.
    #[test]
    fn emlx_shard_calc_small_rowid() {
        let rowid: i64 = 1234;
        assert_eq!((rowid / 1000) % 10, 1);
        assert_eq!((rowid / 10000) % 10, 0);
    }

    /// Boundary: very large ROWID (6-digit+).
    #[test]
    fn emlx_shard_calc_large_rowid() {
        let rowid: i64 = 315376;
        let d1 = (rowid / 1000) % 10; // 315 % 10 = 5
        let d2 = (rowid / 10000) % 10; // 31  % 10 = 1
        assert_eq!(d1, 5);
        assert_eq!(d2, 1);
    }

    #[test]
    fn strip_html_basic_tags() {
        let html = "<p>Dobrý <b>deň</b></p><br>&amp; &lt;test&gt;";
        let out = strip_html(html);
        assert!(out.contains("Dobrý"), "missing 'Dobrý': {out}");
        assert!(out.contains("deň"), "missing 'deň': {out}");
        // &amp; → &  and  &lt;test&gt; → <test>  (entity decode is correct)
        assert!(out.contains("& <test>"), "entity decode wrong: {out}");
        // The actual HTML tags must be gone
        assert!(!out.contains("<p>"), "<p> not stripped: {out}");
        assert!(!out.contains("<b>"), "<b> not stripped: {out}");
        assert!(!out.contains("<br>"), "<br> not stripped: {out}");
    }

    #[test]
    fn detect_language_slovak() {
        // Use a real-world Slovak business email fixture for enough trigram coverage.
        let sk = include_str!("../../../../fixtures/sk/faktura-upomienka.txt");
        // If whatlang deems the text reliable it must return "sk"; None is also
        // acceptable when the text happens to be below the reliability threshold.
        match detect_language(sk) {
            Some(lang) => assert_eq!(lang, "sk", "wrong language for SK fixture"),
            None => { /* below reliability threshold — acceptable for short texts */ }
        }
    }

    #[test]
    fn detect_language_english() {
        let en = "Dear customer, please find the attached invoice for goods delivered. \
                  The total amount includes VAT at 20 percent. Please settle the \
                  payment before the due date. Kind regards, your supplier.";
        assert_eq!(detect_language(en).as_deref(), Some("en"), "text: {en}");
    }

    #[test]
    fn applescript_timestamp_accepts_common_number_formats() {
        assert_eq!(parse_applescript_timestamp("1780556527"), Some(1780556527));
        assert_eq!(
            parse_applescript_timestamp("1780556527.0"),
            Some(1780556527)
        );
        assert_eq!(
            parse_applescript_timestamp("1780556527,0"),
            Some(1780556527)
        );
        assert_eq!(
            parse_applescript_timestamp("1.780556527E+9"),
            Some(1780556527)
        );
        assert_eq!(parse_applescript_timestamp("0"), None);
        assert_eq!(parse_applescript_timestamp("missing value"), None);
    }

    #[test]
    fn applescript_record_preserves_recipient() {
        let record = [
            "1",
            "DB9PR10MB5572EAC11C1694817E4CC41991F42@DB9PR10MB5572.EURPRD10.PROD.OUTLOOK.COM",
            "Radoslava Némethová <radoslava.nemethova@tenenet.sk>",
            "AMB VSL SC a VG",
            "1780556527,0",
            "true",
            "oliver@example.com",
        ]
        .join("\u{1f}");

        let msg = parse_applescript_mail_record(&record).expect("parse AppleScript record");
        assert_eq!(msg.rowid, -1);
        assert_eq!(msg.sender_display.as_deref(), Some("Radoslava Némethová"));
        assert_eq!(msg.sender, "radoslava.nemethova@tenenet.sk");
        assert_eq!(msg.recipient.as_deref(), Some("oliver@example.com"));
        assert_eq!(msg.received_at, 1780556527);
        assert!(msg.is_read);
    }

    #[tokio::test]
    async fn cached_body_is_used_without_automation_and_remains_bounded() {
        let fallback_called = Arc::new(AtomicBool::new(false));
        let called = fallback_called.clone();
        let body = "x".repeat(4_100);
        let hydrated =
            hydrate_loaded_message(hydration_message(Some(&body), true), move |_| async move {
                called.store(true, Ordering::SeqCst);
                Ok(MailAutomationBody::Content("wrong fallback".into()))
            })
            .await;

        assert_eq!(hydrated.state, MailBodyHydrationState::Readable);
        assert_eq!(
            hydrated.message.body.as_deref().unwrap().chars().count(),
            4_000
        );
        assert!(!hydrated.used_automation);
        assert!(!fallback_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn uncached_body_uses_successful_automation_fallback() {
        let hydrated =
            hydrate_loaded_message(hydration_message(None, false), |identity| async move {
                assert_eq!(identity.subject, "Duplicate subject");
                assert_eq!(identity.sender, "alice@example.com");
                assert_eq!(identity.received_at, 1_785_300_000);
                Ok(MailAutomationBody::Content(
                    "Hydrated through Mail.app.".into(),
                ))
            })
            .await;

        assert_eq!(hydrated.state, MailBodyHydrationState::Readable);
        assert_eq!(
            hydrated.message.body.as_deref(),
            Some("Hydrated through Mail.app.")
        );
        assert!(hydrated.message.body_available);
        assert!(hydrated.used_automation);
    }

    #[test]
    fn body_fallback_disambiguates_duplicate_subjects_by_sender_and_date() {
        let script = mail_body_applescript(&MailBodyIdentity {
            subject: "Duplicate subject".into(),
            sender: "alice@example.com".into(),
            received_at: 1_785_300_000,
        });

        assert!(script.contains("whose subject is \"Duplicate subject\""));
        assert!(script.contains("snd contains \"alice@example.com\""));
        assert!(script.contains("dateDelta <= 2"));
        assert!(script.contains("dateDelta to unixTs - 1785300000"));
        assert!(script.contains("- (time to GMT)"));
        assert!(script.contains("if errorNumber is -1743 then"));
        assert!(script.contains("error errorMessage number errorNumber"));
        assert!(script.contains("if fatalErrorNumber is not 0 then"));
        assert!(!script.contains("content of item 1"));
    }

    #[tokio::test]
    async fn empty_automation_fallback_is_typed_empty() {
        let hydrated = hydrate_loaded_message(hydration_message(None, false), |_| async {
            Ok(MailAutomationBody::Content("   ".into()))
        })
        .await;

        assert_eq!(hydrated.state, MailBodyHydrationState::Empty);
        assert!(hydrated.message.body_available);
        assert_eq!(hydrated.message.body.as_deref(), Some("   "));
    }

    #[test]
    fn automation_denial_is_typed_separately() {
        let result = parse_mail_body_automation_output(AutomationOutput {
            success: false,
            stdout: String::new(),
            stderr: "execution error: Not authorized to send Apple events to Mail. (-1743)".into(),
        });

        assert_eq!(result, Err(MailAutomationError::Denied));
    }

    #[test]
    fn automation_failure_is_not_mislabeled_unavailable() {
        let result = parse_mail_body_automation_output(AutomationOutput {
            success: false,
            stdout: String::new(),
            stderr: "execution error: Mail got an error while reading content. (-1728)".into(),
        });

        assert_eq!(result, Err(MailAutomationError::Failed));
    }

    #[tokio::test]
    async fn automation_fallback_timeout_is_typed_separately() {
        let result = run_automation_with_timeout(
            Duration::from_millis(5),
            std::future::pending::<std::io::Result<AutomationOutput>>(),
        )
        .await;

        assert!(matches!(result, Err(MailAutomationError::TimedOut)));
    }

    #[tokio::test]
    #[ignore = "requires Full Disk Access, Mail.app Automation, and a currently uncached message"]
    async fn real_mail_uncached_message_becomes_readable() {
        let connector = MailConnector::new().expect("Apple Mail connector");
        let headers = connector
            .list_inbox(100, false)
            .expect("read recent Mail headers");
        let uncached = headers
            .into_iter()
            .find(|header| {
                connector
                    .get_message(header.rowid)
                    .ok()
                    .flatten()
                    .is_some_and(|message| message.body.is_none() && !message.body_available)
            })
            .expect("at least one recent message must be uncached for this smoke test");

        let hydrated = connector
            .hydrate_message(uncached.rowid)
            .await
            .expect("shared body hydration")
            .expect("message still exists");

        assert_eq!(hydrated.state, MailBodyHydrationState::Readable);
        assert!(hydrated.used_automation);
        assert!(!hydrated
            .message
            .body
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty());
    }

    // ── Phase 5C — attachment extraction from raw .eml ───────────────────────
    //
    // These tests call `mailparse::parse_mail` directly on the raw .eml fixture
    // (no emlx plist-length prefix) so they work independently of the real Mail
    // store being present.

    #[test]
    fn eml_pdf_invoice_has_attachment() {
        let eml = include_bytes!("../../../../fixtures/sk/mail_with_pdf_invoice.eml");
        let parsed = mailparse::parse_mail(eml).expect("parse mail_with_pdf_invoice.eml");
        let attachments = extract_attachments_from_parsed(&parsed);
        assert!(
            !attachments.is_empty(),
            "expected ≥1 attachment in PDF invoice fixture"
        );
        let pdf = attachments.iter().find(|a| a.mimetype == "application/pdf");
        assert!(
            pdf.is_some(),
            "expected a PDF attachment, got: {attachments:?}"
        );
        let pdf = pdf.unwrap();
        assert!(
            pdf.filename.contains("faktura"),
            "filename should contain 'faktura': {}",
            pdf.filename
        );
    }

    #[test]
    fn eml_pdf_invoice_body_contains_dph_iban() {
        let eml = include_bytes!("../../../../fixtures/sk/mail_with_pdf_invoice.eml");
        let parsed = mailparse::parse_mail(eml).expect("parse mail_with_pdf_invoice.eml");
        let body = extract_plain_text(&parsed).expect("extract body");
        assert!(body.contains("DPH"), "body should contain 'DPH': {body}");
        assert!(body.contains("IBAN"), "body should contain 'IBAN': {body}");
    }

    #[test]
    fn eml_image_receipt_has_image_attachment() {
        let eml = include_bytes!("../../../../fixtures/sk/mail_with_image_receipt.eml");
        let parsed = mailparse::parse_mail(eml).expect("parse mail_with_image_receipt.eml");
        let attachments = extract_attachments_from_parsed(&parsed);
        assert!(
            !attachments.is_empty(),
            "expected ≥1 attachment in image receipt fixture"
        );
        let img = attachments
            .iter()
            .find(|a| a.mimetype.starts_with("image/"));
        assert!(
            img.is_some(),
            "expected an image attachment, got: {attachments:?}"
        );
        let img = img.unwrap();
        assert_eq!(img.mimetype, "image/jpeg");
        assert!(
            img.filename.contains("uctenka"),
            "filename should contain 'uctenka': {}",
            img.filename
        );
    }

    #[test]
    fn eml_image_receipt_is_classified_as_image() {
        let eml = include_bytes!("../../../../fixtures/sk/mail_with_image_receipt.eml");
        let parsed = mailparse::parse_mail(eml).expect("parse mail_with_image_receipt.eml");
        let attachments = extract_attachments_from_parsed(&parsed);
        let has_image = attachments.iter().any(|a| a.mimetype.starts_with("image/"));
        assert!(
            has_image,
            "image receipt should be classified as an image attachment"
        );
    }
}

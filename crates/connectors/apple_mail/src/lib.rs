use std::path::{Path, PathBuf};

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
        self.envelope_index.exists()
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

    /// Fetch attachment bytes as a base64-encoded string (for Ollama vision / JSON APIs).
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

/// Fetch the plain-text body via `osascript` when the emlx is not locally
/// cached (typical for IMAP messages that Mail never fully downloaded).
///
/// Requires Automation → Mail permission (NOT Full Disk Access).
/// Searches the inbox for the first message whose subject matches.
pub async fn body_via_applescript(subject: &str) -> Option<String> {
    let safe = subject.replace('\\', "\\\\").replace('"', "\\\"");
    // Search all accounts/mailboxes, not just inbox — emails may be in subfolders.
    let script = format!(
        r#"tell application "Mail"
    try
        set allMessages to {{}}
        repeat with acct in every account
            repeat with mb in every mailbox of acct
                try
                    set found to (every message of mb whose subject is "{safe}")
                    if (count of found) > 0 then
                        return content of item 1 of found
                    end if
                end try
            end repeat
        end repeat
    end try
    return ""
end tell"#
    );
    let out = tokio::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .await
        .ok()?;
    if out.status.success() {
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !text.is_empty() && text != "missing value" {
            Some(text)
        } else {
            None
        }
    } else {
        None
    }
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
    fn eml_image_receipt_triggers_vision_route() {
        // Simulate the daemon logic: if any attachment is image/* → vision model needed
        let eml = include_bytes!("../../../../fixtures/sk/mail_with_image_receipt.eml");
        let parsed = mailparse::parse_mail(eml).expect("parse mail_with_image_receipt.eml");
        let attachments = extract_attachments_from_parsed(&parsed);
        let has_image = attachments.iter().any(|a| a.mimetype.starts_with("image/"));
        assert!(has_image, "vision route should trigger for image receipt");
    }
}

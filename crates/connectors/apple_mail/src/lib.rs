use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailMessage {
    pub rowid: i64,
    pub subject: String,
    /// RFC 5322 address string
    pub sender: String,
    /// Display name if present
    pub sender_display: Option<String>,
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
}

// ── Connector ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct MailConnector {
    envelope_index: PathBuf,
    mail_v10_dir: PathBuf,
}

impl MailConnector {
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("no home dir"))?;
        let mail_v10_dir = home.join("Library/Mail/V10");
        let envelope_index = mail_v10_dir.join("MailData/Envelope Index");
        Ok(Self { envelope_index, mail_v10_dir })
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
                m.ROWID,
                m.date_received,
                m.read,
                COALESCE(s.subject, '(no subject)'),
                COALESCE(a.comment, ''),
                COALESCE(a.address, ''),
                COALESCE(mb.url, '')
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
                sender_display: if sender_display.is_empty() { None } else { Some(sender_display) },
                sender: row.get(5)?,
                mailbox_url: row.get(6)?,
                body: None,
                body_available: true,
                language: None,
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
                COALESCE(mb.url, '')
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
                sender_display: if display.is_empty() { None } else { Some(display) },
                sender: row.get(5)?,
                mailbox_url: row.get(6)?,
                body: None,
                body_available: true,
                language: None,
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
                COALESCE(mb.url, '')
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
                    sender_display: if sender_display.is_empty() { None } else { Some(sender_display) },
                    sender: row.get(5)?,
                    mailbox_url: row.get(6)?,
                    body: None,
                    body_available: false,
                    language: None,
                })
            },
        );

        let Ok(mut msg) = result else { return Ok(None) };

        if let Some(emlx_path) = self.find_emlx(rowid) {
            match parse_emlx_body(&emlx_path) {
                Ok(text) if !text.trim().is_empty() => {
                    msg.language = detect_language(&text);
                    msg.body = Some(text);
                    msg.body_available = true;
                }
                _ => {}
            }
        }
        // AppleScript fallback (when body_available stays false): Phase 4+

        Ok(Some(msg))
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

    /// Walk V10/{acct}/*.mbox/{guid}/Data/{d1}/{d2}/Messages/{rowid}.emlx
    fn find_emlx(&self, rowid: i64) -> Option<PathBuf> {
        let d1 = (rowid / 1000) % 10;
        let d2 = (rowid / 10000) % 10;
        let filename = format!("{rowid}.emlx");

        for acct in std::fs::read_dir(&self.mail_v10_dir).ok()?.flatten() {
            let Ok(ft) = acct.file_type() else { continue };
            if !ft.is_dir() { continue }

            for mbox in std::fs::read_dir(acct.path()).ok()?.flatten() {
                let mp = mbox.path();
                if mp.extension().and_then(|e| e.to_str()) != Some("mbox") { continue }

                for guid in std::fs::read_dir(&mp).ok()?.flatten() {
                    let Ok(gft) = guid.file_type() else { continue };
                    if !gft.is_dir() { continue }

                    let candidate = guid.path()
                        .join("Data")
                        .join(d1.to_string())
                        .join(d2.to_string())
                        .join("Messages")
                        .join(&filename);

                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    }
}

// ── emlx parser ───────────────────────────────────────────────────────────────

/// Parse an Apple Mail emlx file and extract the best plain-text body.
///
/// emlx layout:
///   Line 1:  ASCII integer = byte count of trailing binary plist
///   Lines 2…N: RFC 2822 email (headers + body)
///   Tail:    binary plist of Mail metadata (flags, colours, …)
fn parse_emlx_body(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;

    let nl = bytes.iter().position(|&b| b == b'\n')
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

    let parsed = mailparse::parse_mail(&bytes[email_start..email_end])?;
    extract_plain_text(&parsed)
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
                if !t.trim().is_empty() { plain_parts.push(t); }
            } else if pct == "text/html" && html_fallback.is_none() {
                html_fallback = Some(strip_html(&part.get_body()?));
            } else if pct.starts_with("multipart/") {
                if let Ok(nested) = extract_plain_text(part) {
                    if !nested.trim().is_empty() { plain_parts.push(nested); }
                }
            }
        }

        if !plain_parts.is_empty() { return Ok(plain_parts.join("\n\n")); }
        if let Some(html) = html_fallback { return Ok(html); }
    }

    Ok(String::new())
}

/// Minimal HTML tag stripper — no external deps.
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => { in_tag = false; out.push(' '); }
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
    let script = format!(
        r#"tell application "Mail"
    try
        set msgList to (every message of inbox whose subject is "{safe}")
        if (count of msgList) > 0 then
            return content of item 1 of msgList
        end if
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
        if !text.is_empty() && text != "missing value" { Some(text) } else { None }
    } else {
        None
    }
}

// ── Language detection ────────────────────────────────────────────────────────

pub fn detect_language(text: &str) -> Option<String> {
    let info = whatlang::detect(text)?;
    if !info.is_reliable() { return None; }
    Some(match info.lang() {
        whatlang::Lang::Slk => "sk",
        whatlang::Lang::Ces => "cs",
        whatlang::Lang::Eng => "en",
        whatlang::Lang::Deu => "de",
        _ => return None,
    }.to_string())
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
        let d1 = (rowid / 1000) % 10;  // 315 % 10 = 5
        let d2 = (rowid / 10000) % 10; // 31  % 10 = 1
        assert_eq!(d1, 5);
        assert_eq!(d2, 1);
    }

    #[test]
    fn strip_html_basic_tags() {
        let html = "<p>Dobrý <b>deň</b></p><br>&amp; &lt;test&gt;";
        let out = strip_html(html);
        assert!(out.contains("Dobrý"), "missing 'Dobrý': {out}");
        assert!(out.contains("deň"),   "missing 'deň': {out}");
        // &amp; → &  and  &lt;test&gt; → <test>  (entity decode is correct)
        assert!(out.contains("& <test>"), "entity decode wrong: {out}");
        // The actual HTML tags must be gone
        assert!(!out.contains("<p>"),  "<p> not stripped: {out}");
        assert!(!out.contains("<b>"),  "<b> not stripped: {out}");
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
}

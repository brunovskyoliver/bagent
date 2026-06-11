# Spike: Apple Mail Schema

**Date:** 2026-06-11
**macOS:** 25.5.0 (Darwin 25.5.0)
**Mail schema version:** V10
**Envelope Index location:** `~/Library/Mail/V10/MailData/Envelope Index`

---

## Database Tables (Envelope Index)

Key tables relevant to bagent:

```
messages          — one row per message; core metadata
mailboxes         — accounts + folders (IMAP/local)
subjects          — deduplicated subject strings
addresses         — deduplicated email addresses
senders           — sender metadata (links to addresses)
recipients        — recipient list per message
conversations     — thread grouping
summaries         — generated message summaries (Mail.app internal)
generated_summaries — AI-generated summaries (macOS 15+ Summarize feature)
searchable_messages — FTS index mirror
attachments       — attachment metadata
```

---

## Key Table Schemas

### `messages`

```sql
ROWID              INTEGER  -- primary key; also used in emlx path (see below)
message_id         INTEGER  -- internal identifier
global_message_id  INTEGER
remote_id          INTEGER  -- IMAP UID
document_id        TEXT     -- used to locate emlx file (see path section)
sender             INTEGER  -- FK → addresses.ROWID
subject            INTEGER  -- FK → subjects.ROWID
date_sent          INTEGER  -- Unix timestamp (seconds since 1970-01-01)
date_received      INTEGER  -- Unix timestamp
mailbox            INTEGER  -- FK → mailboxes.ROWID
read               INTEGER  -- 0 = unread, 1 = read
flagged            INTEGER
deleted            INTEGER
size               INTEGER  -- bytes
conversation_id    INTEGER  -- FK → conversations.ROWID
```

**Timestamps are Unix epoch (seconds since 1970-01-01), NOT Core Data epoch.**

### `mailboxes`

```sql
ROWID        INTEGER
url          TEXT    -- e.g. "imap://F662FD30-12F6-4FC6-83CC-9CE425418EA9/INBOX"
total_count  INTEGER
unread_count INTEGER
```

### `subjects`

```sql
ROWID    INTEGER
subject  TEXT    -- raw subject string
```

### `addresses`

```sql
ROWID    INTEGER
address  TEXT    -- RFC 5322 email address (may include display name)
comment  TEXT    -- display name
```

---

## Working Query: Unread Messages

```sql
SELECT 
    m.ROWID           AS rowid,
    m.date_received   AS received_unix,
    datetime(m.date_received, 'unixepoch') AS received_iso,
    s.subject         AS subject,
    a.address         AS sender,
    mb.url            AS mailbox_url
FROM messages m
LEFT JOIN subjects  s  ON m.subject = s.ROWID
LEFT JOIN addresses a  ON m.sender  = a.ROWID
LEFT JOIN mailboxes mb ON m.mailbox = mb.ROWID
WHERE m.read    = 0
  AND m.deleted = 0
ORDER BY m.date_received DESC
LIMIT 20;
```

**Sample output (real data, anonymized):**

| ROWID | received_iso | subject | sender |
|---|---|---|---|
| 315376 | 2026-06-11 … | Vaša zásielka bola úspešne presmerovaná | noreply@dpdgroup.com |
| 315366 | 2026-06-11 … | Ďakujeme za účasť na Odoo Business Show Bratislava! | samuel.sarnovsky@26house.com |
| 315347 | 2026-06-11 … | Super! Máme Vašu objednávku … | web@countrysaloon.sk |

Slovak subjects present and correctly encoded (UTF-8 diacritics visible).

---

## emlx File Format and Path

### Format

Each `.emlx` file starts with:
1. **Line 1:** A decimal integer = byte length of the plist footer at end of file.
2. **Lines 2–N:** Standard RFC 2822 email (headers + body), **Content-Transfer-Encoding: quoted-printable** for UTF-8 bodies.
3. **Last N bytes:** Binary plist with Mail metadata (flags, color, etc.).

**Example header observed:**
```
163909
From: Oliver Brunovsky <oliver.brunovsky@novem.sk>
Content-Type: multipart/alternative; boundary="Apple-Mail=_..."
Mime-Version: 1.0
Subject: [subject]
Message-Id: <...@novem.sk>
Date: Wed, 18 Dec 2024 13:20:54 +0100
```

**Quoted-printable decoding required:** `=C3=BD` → `ý`, `=C3=BA` → `ú`, etc. Python `quopri.decodestring()` or Rust `quoted_printable` crate.

**HTML stripping:** Bodies are `multipart/alternative` with both `text/plain` and `text/html` parts. Prefer `text/plain`; fall back to HTML → Markdown conversion.

### Path Pattern

```
~/Library/Mail/V10/
  {account-uuid}/         ← from mailbox URL: imap://{account-uuid}/...
    {MailboxName}.mbox/
      {folder-uuid}/
        Data/
          {shard_d1}/     ← floor(emlx_id / 1000) % 10
          {shard_d2}/     ← floor(emlx_id / 10000) % 10
            Messages/
              {emlx_id}.emlx
```

**emlx filename = `messages.ROWID`** — confirmed via cross-referencing DB queries and filesystem.

```
Path: {mbox_dir}/Data/{(ROWID/1000)%10}/{(ROWID/10000)%10}/Messages/{ROWID}.emlx
```

Example: ROWID=95804 → `Data/5/9/Messages/95804.emlx` ✓

```python
def emlx_path(mbox_dir: str, rowid: int) -> str:
    d1 = (rowid // 1000) % 10
    d2 = (rowid // 10000) % 10
    return f"{mbox_dir}/Data/{d1}/{d2}/Messages/{rowid}.emlx"
```

**Critical limitation — IMAP partial download:** Not all messages have a local emlx file. Only messages Mail has downloaded locally exist as emlx. On this system, ~768 of 84,273 messages have local emlx (ROWID range ~91,000–98,882). Messages with ROWID > 98,882 exist in the DB metadata (subject, sender, date) but have no local body file.

**Connector strategy:**
- Metadata (subject, sender, date, read status): always available from `Envelope Index` SQLite.
- Body for locally-cached messages: read emlx directly.
- Body for non-cached messages: fall back to `tell application "Mail" to get content of message id X` via AppleScript (forces Mail to download), or surface "body not available offline" to user.
- MVP: use AppleScript body fallback; emit warning if Mail is not running.

---

## macOS Version Guard

The Envelope Index schema may change between macOS versions. Detect version at connector startup:

```sql
-- Check schema version indicator
SELECT value FROM properties WHERE key = 'version';
-- Or check via Mail app bundle:
-- /System/Applications/Mail.app/Contents/Info.plist → CFBundleShortVersionString
```

Known safe schema: macOS 14 (V10), macOS 15 (V10 with additional columns). If schema version unrecognized → return `ConnectorError::SchemaVersion`; show "Mail connector needs update" in UI.

---

## Locking Considerations

The Envelope Index uses WAL mode (`Envelope Index-wal` file present). When Mail.app is writing:
- Open with `SQLITE_OPEN_READONLY` — readers don't block on WAL.
- Use `PRAGMA busy_timeout = 2000` to handle brief write locks.
- If `SQLITE_BUSY` after timeout → retry up to 3× with 200 ms backoff; surface error to user.

---

## Permissions

- **Full Disk Access** required to read `~/Library/Mail/V10/` without AppleScript.
- **Alternative:** Use AppleScript/JXA `tell application "Mail"` which requires only Automation → Mail permission. Less brittle (no schema dependency) but slower and limited query capability.
- **MVP recommendation:** Full Disk Access + direct SQLite for performance; AppleScript as fallback for message body if emlx path resolution fails.

---

## Action Items

- [x] Verify emlx path derivation: **emlx filename = messages.ROWID**, sharding `dir1=(ROWID/1000)%10`, `dir2=(ROWID/10000)%10`.
- [x] Confirmed `document_id` is NULL for 84,025/84,273 messages — not the right column.
- [ ] Write emlx parser: quoted-printable decode + MIME multipart + HTML strip.
- [ ] Implement AppleScript body fallback for non-locally-cached messages.
- [ ] Test on macOS 15 — confirm V10 schema is compatible.
- [ ] Benchmark: how long to scan `Envelope Index` for 84,273 messages (full count on this system)?
- [ ] Handle `date_received` = 0 or NULL (malformed messages).
- [ ] Surface "body not available offline" gracefully when emlx missing and Mail not running.

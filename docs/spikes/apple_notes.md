# Spike: Apple Notes Schema

**Date:** 2026-06-11
**macOS:** 25.5.0 (Darwin 25.5.0)
**Database:** `~/Library/Group Containers/group.com.apple.notes/NoteStore.sqlite`

---

## Database Tables

```
ZICCLOUDSYNCINGOBJECT  — main polymorphic table: notes, folders, accounts, attachments (all entity types)
ZICNOTEDATA            — note body data (protobuf-encoded rich text + ZMERGEABLEDATA)
Z_METADATA             — Core Data metadata
Z_PRIMARYKEY           — entity type mapping
ATRANSACTION           — CloudKit sync transactions
ACHANGE                — change tracking
```

The Core Data model uses a **single-table inheritance** pattern. All entities (notes, folders, attachments, accounts) share `ZICCLOUDSYNCINGOBJECT`. Entity type is determined by `Z_ENT`.

---

## Entity Type Mapping

```sql
SELECT * FROM Z_PRIMARYKEY;
```

Check `Z_NAME` to find the entity numbers for:
- `ICNote` — note entity
- `ICFolder` — folder entity
- `ICAccount` — account entity
- `ICAttachment` — attachment entity

Use `Z_ENT` filter in queries to avoid mixing entity types.

---

## Key Columns in `ZICCLOUDSYNCINGOBJECT`

### For Notes (`ICNote` entity)

| Column | Type | Notes |
|---|---|---|
| Z_PK | INTEGER | Primary key |
| Z_ENT | INTEGER | Entity type (find via Z_PRIMARYKEY) |
| ZTITLE | VARCHAR | System-computed title (from first line of body) |
| ZUSERTITLE | VARCHAR | User-set title (if explicitly named) |
| ZSUMMARY | VARCHAR | System-generated snippet |
| ZCREATIONDATE | TIMESTAMP | Core Data epoch (add 978307200 for Unix) |
| ZMODIFICATIONDATE | TIMESTAMP | Core Data epoch |
| ZNOTEDATA | INTEGER | FK → ZICNOTEDATA.Z_PK |
| ZFOLDER | INTEGER | FK → parent folder object |
| ZMARKEDFORDELETION | INTEGER | 1 = in trash |
| ZISPINNED | INTEGER | 1 = pinned note |
| ZISPASSWORDPROTECTED | INTEGER | 1 = locked (body encrypted) |
| ZMERGEABLEDATA | BLOB | CloudKit operational transform data |
| ZNOTEDATA (in ZICNOTEDATA) | BLOB | Rich text body as protobuf |

**Core Data epoch:** `unix_timestamp = cd_timestamp + 978307200` where 978307200 = seconds between 1970-01-01 and 2001-01-01.

### For Folders (`ICFolder` entity)

| Column | Notes |
|---|---|
| ZNAME | Folder name |
| ZPARENT | FK → parent folder (NULL for top-level) |
| ZFOLDERTYPE | 0=normal, 1=smart, 2=recently-deleted |

---

## Working Queries

### List Notes (text notes only, not in trash)

```sql
-- First find the ICNote entity number
SELECT Z_ENT, Z_NAME FROM Z_PRIMARYKEY WHERE Z_NAME = 'ICNote';
-- → typically Z_ENT = 9 or 10 (varies by OS version)

-- Then query notes
SELECT 
    n.Z_PK,
    COALESCE(n.ZUSERTITLE, n.ZTITLE) AS title,
    n.ZSUMMARY AS snippet,
    datetime(n.ZMODIFICATIONDATE + 978307200, 'unixepoch') AS modified,
    datetime(n.ZCREATIONDATE + 978307200, 'unixepoch') AS created,
    n.ZISPINNED,
    n.ZISPASSWORDPROTECTED
FROM ZICCLOUDSYNCINGOBJECT n
WHERE n.Z_ENT = (SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME = 'ICNote')
  AND n.ZMARKEDFORDELETION = 0
  AND n.ZNOTEDATA IS NOT NULL
ORDER BY n.ZMODIFICATIONDATE DESC
LIMIT 20;
```

### List Folders

```sql
SELECT 
    ZNAME,
    datetime(ZMODIFICATIONDATE + 978307200, 'unixepoch') AS modified
FROM ZICCLOUDSYNCINGOBJECT
WHERE Z_ENT = (SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME = 'ICFolder')
  AND ZMARKEDFORDELETION = 0
ORDER BY ZNAME;
```

---

## Note Body Extraction

The note body is stored in `ZICNOTEDATA.ZNOTEDATA` as a **serialized protobuf** (Apple's internal `com.apple.mobilenotes.SharingService` protobuf format). It is **not** plain text or HTML.

### Options

1. **AppleScript / JXA (recommended for MVP):** Returns plain text or HTML body without needing to decode protobuf.

```javascript
// JXA
const notes = Application("Notes")
const note = notes.notes.byId("x-coredata://...")
console.log(note.body())  // returns HTML string
```

2. **Direct protobuf decode (complex, fragile):** The protobuf schema is undocumented and changes between OS versions. Several community projects have reverse-engineered it (e.g. `notes-python`, `apple-notes-to-sqlite`). Not recommended for production; too brittle.

3. **`ZSUMMARY` / `ZADDITIONALINDEXABLETEXT` columns:** Available as plain-text fallback for snippet/search. Not the full body but sufficient for search indexing.

**MVP recommendation:** Use AppleScript/JXA for body retrieval. Cache plain text in `notes` table after first fetch. Use `ZSUMMARY` for lightweight search without AppleScript overhead.

---

## AppleScript / JXA Path

```javascript
// JXA: list all notes with title + body
const app = Application("Notes")
app.includeStandardAdditions = true

const result = []
app.folders().forEach(folder => {
    folder.notes().forEach(note => {
        result.push({
            id: note.id(),
            title: note.name(),
            body: note.body(),          // HTML
            creationDate: note.creationDate(),
            modificationDate: note.modificationDate(),
            container: folder.name()
        })
    })
})
JSON.stringify(result)
```

Run via: `osascript -l JavaScript -e '...'`

**Permission required:** Automation → Notes (in System Settings → Privacy & Security → Automation).

---

## Password-Protected Notes

Notes with `ZISPASSWORDPROTECTED = 1` have encrypted body in `ZCRYPTOWRAPPEDKEY` / `ZCRYPTOINITIALIZATIONVECTOR` / `ZCRYPTOTAG` columns. Cannot be decrypted without the user's Notes password or device unlock.

**MVP behavior:** Skip locked notes; surface count of locked notes to user ("3 notes are locked and cannot be read"). Never attempt to decrypt.

---

## Sync State

Notes sync via CloudKit (iCloud). The local database may lag behind iCloud by seconds to minutes:
- `ZNEEDSTOBEFETCHEDFROMCLOUD = 1` → note body not yet downloaded.
- `ZNEEDSINITIALFETCHFROMCLOUD = 1` → first-time setup incomplete.

**Handling:** If body is unavailable locally, use `ZSUMMARY` as fallback; do not retry CloudKit fetch (that's the OS's job).

---

## Action Items

- [ ] Identify `Z_ENT` values for `ICNote` and `ICFolder` on this macOS version.
- [ ] Test JXA body retrieval for 5 sample notes; verify SK diacritics preserved.
- [ ] Benchmark: JXA list-all-notes on a Notes vault with ~200 notes (expected: 1–3 s).
- [ ] Implement `ZSUMMARY` fallback for search when JXA is unavailable.
- [ ] Document behavior for password-protected notes.
- [ ] Verify `ZMODIFICATIONDATE` Core Data epoch conversion (+978307200) gives correct dates.
- [ ] Test on a Notes vault with iCloud sync active — check for race conditions during background sync.

# Spike: reading macOS Notification Center

Verified empirically on this machine, 2026-08-20. macOS build `24A348`,
`dbinfo.version` = 19, `compatibleVersion` = 17.

## Access

`~/Library/Group Containers/group.com.apple.usernoted/db2/db`

TCC-protected, in the same class as `~/Library/Mail` and `~/Library/Messages`.
Without a grant the denial lands at the TCC layer before any file is opened, so
even `ls` on the parent returns `Operation not permitted` — it is not a
file-mode problem and not a process-sandbox problem (verified: identical failure
with tool sandboxing disabled, while sibling group containers list normally).

Full Disk Access, granted to the responsible app of whichever process reads it,
is sufficient. No entitlement is involved. Once granted, reads work immediately.

Unverified: whether a launchd-run `bagentd` can hold its own FDA grant, and
whether the grant on `bagent.app` extends to the daemon it spawns. This still
needs testing in both `make run` and `make bundle`.

## Schema

```sql
app(app_id INTEGER PRIMARY KEY, identifier VARCHAR, badge INTEGER)
record(rec_id INTEGER PRIMARY KEY, app_id INTEGER, uuid BLOB, data BLOB,
       request_date REAL, request_last_date REAL, delivered_date REAL,
       presented BOOL, style INTEGER, snooze_fire_date REAL)
requests / delivered / displayed / snoozed / categories (app_id, list BLOB)
dbinfo(key, value)
```

A trigger on `app` deletion cascades to every other table.

`record.uuid` was unique across all rows (35/35) and is what the mirror uses as
`source_id`. The `delivered` / `displayed` list blobs decoded to nothing useful;
`record` is the only table worth reading.

## The `data` blob

Binary plist. Key frequency across all 35 live records:

| key | type | present | meaning |
|---|---|---|---|
| `app` | str | 35/35 | bundle identifier |
| `date` | float | 34/35 | CFAbsoluteTime (epoch 2001-01-01), +978307200 → Unix |
| `req.titl` | str | 35/35 | title |
| `req.subt` | str | 26/35 | subtitle |
| `req.body` | str | **12/35** | body — absent more often than present |
| `req.thre` | str | **2/35** | thread identifier — rare |
| `req.cate` | str | 2/35 | category |
| `req.iden` | str | 7/35 | request identifier |
| `req.durl` | str | 2/35 | destination URL |

Also present and unused: `req.scat` (category actions), `resp` (user response),
`req.atta` (attachments), `req.soun`, `styl`, `orig`, `srce`.

## The finding that matters

**There is no history.** The database holds only what Notification Center is
currently showing. 35 rows spanning 19 months, of which 29 fell in the last
three days; the older survivors are sticky notifications never dismissed
(AdGuard, yabai, Time Machine, one mail client). Dismissing a notification
deletes its row.

Consequences for the plan:

- Decision 3's premise was wrong. The DB is a live snapshot, not an archive.
- Decision 4 (mirror) is therefore load-bearing rather than an optimisation.
  bagent's history is exactly what its poll loop managed to observe.
- Anything delivered and dismissed between two polls is lost permanently. At a
  30s cadence this is a small but real hole, and it is unfixable from this
  interface.
- Decision 11 survives but is mostly inert: with `req.thre` on 2 of 35 records,
  nearly every notification is its own group. The implementation already treats
  a missing thread id as "do not collapse", which is now the dominant path
  rather than the edge case. Note the sample contains no Slack or Messages —
  the apps most likely to thread — so the ratio may differ in practice.

## Approaches

| Approach | Yields | Permission | Fragility | macOS 26 |
|---|---|---|---|---|
| `usernoted` DB | Currently-displayed notifications, all apps | Full Disk Access | Private schema, undocumented plist keys | Works (verified, build 24A348) |
| `UNUserNotificationCenter` | Only the calling app's own notifications | None | Stable, supported | Useless here |
| Accessibility scraping | Same set as the DB, worse fidelity | Accessibility | UI-dependent | Not tested |

The DB is the right choice. It yields strictly more than the alternatives at a
permission cost the user has already accepted.

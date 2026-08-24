# bagent Notifications — implementation plan

Status: implemented and verified against the live system. Remaining work is the
Swift settings toggle. See `docs/spikes/notification_access.md`.

## Goal

bagent can recall and reason about macOS notifications the user received, on
demand, without notification text degrading the model's answers or acting as an
instruction channel.

## Decisions

| # | Decision | Choice |
|---|---|---|
| 1 | Purpose | Cover connector blind spots only (Messages, Slack, Calendar, Teams, banking, system alerts). Mail/WhatsApp banners are dropped — the connectors answer those better. |
| 2 | Injection | Tool-only. Never auto-injected into the system prompt. Preserves the existing `stateless_no_recall` prompt invariant. |
| 3 | Capture | Read `~/Library/Group Containers/group.com.apple.usernoted/db2/db`. Requires Full Disk Access. **Revised after the spike:** that database holds only what Notification Center is currently showing, not history — dismissed notifications are deleted. |
| 4 | Storage | Mirror into bagent's SQLite. Load-bearing, not an optimisation: bagent's history is exactly what its poll loop observed. Notifications delivered and dismissed between polls are lost permanently. |
| 5 | Filtering | Hardcoded bundle-id denylist at ingest, next to the connector registry. |
| 6 | Trust | Results are wrapped, labelled untrusted third-party text, carry app + exact timestamp, and can never authorize a side-effecting tool call — enforced in the dispatcher, not by prompt wording. |
| 7 | Retention | 30-day rolling purge on each poll. "Forget all notifications" button in settings. |
| 8 | Tool surface | One tool: `notifications_search(query?, app?, since?, until?, limit?)`. Rules action `notifications.search`, default `auto` (read-only). |
| 9 | Owner | `bagentd` collects directly. It already runs as a LaunchAgent (`DaemonLauncher.swift`) and survives app exit, so the mirror stays current with the app quit. Swift is out of the ingest path entirely — no provider, no POST endpoint. Full Disk Access is granted to the `bagentd` binary; bplist decoded in Rust via the `plist` crate. |
| 10 | Automations | Allowed. Read-only, no new rules. Unattended-safety rule (fresh approval for side-effecting tools) still applies. |
| 11 | Shaping | Collapse by `(app, thread_id)`; return latest body + count + time span. Cap 20 groups per call, and 3 per app unless the query named one app. **Spike note:** `req.thre` appeared on only 2 of 35 live records, so "one row, one group" is the dominant path; without the per-app cap a single unthreaded app filled all 20 slots on real data. |
| 12 | Degradation | Fail closed. Schema validated on poller startup; mismatch → stop ingesting, mark feed unavailable, tool returns structured `notifications_unavailable` (same shape as `browser_disabled`). Stale mirror stays queryable, every result carries its as-of timestamp. |

Defaulted without discussion, by precedent:
- Off by default behind an explicit notch settings toggle (same as bagent Browser).
- 30s poll in the daemon whenever the LaunchAgent is alive, independent of the app.

## Known risk

TCC attribution for `bagentd` differs between `make run` (spawned by the Swift
app / terminal, grant follows the responsible parent) and the launchd agent
(binary is its own responsible process, needs its own Full Disk Access grant).
This must be proven empirically before any collector code is written — it is
the first ticket, and it can invalidate decision 9.

## Open — blocked on research spike

- Does the DB hold delivered history on macOS 26, or only pending?
- Apple-side retention/eviction: does clearing Notification Center delete rows?
- `record` BLOB (bplist) key layout: bundle id, title, subtitle, body, date, thread id.
- Whether Full Disk Access is sufficient, or a group-container entitlement is also needed.
- Whether a launchd-run bare binary can hold an FDA grant at all, or whether the collector must live inside the `.app`.

## Work breakdown

Done:

- `crates/daemon/migrations/V18__notifications.sql` — `notifications` table,
  `(app_bundle_id, thread_id)` index, `source_id` unique for re-poll dedupe,
  `notifications_state` for the collector watermark, `connectors` seed row
  (`kind='notifications'`, `enabled=0`). Two folded columns, `search_text` and
  `app_text`: SQLite's `lower()` and `LIKE` are ASCII-only, so `'ČAU' LIKE
  '%čau%'` is false and case folding has to happen in Rust for Slovak to be
  searchable at all.
- `crates/daemon/src/notifications.rs` — search, thread collapsing, untrusted
  render, fail-closed availability check, denylist, plus the collector's write
  half: `ingest` (denylist at write, idempotent on `source_id`), `purge`
  (30-day, falls back to `ingested_at` when delivery time is missing) and
  `mark_synced`. Unparseable `since`/`until` returns a corrective tool result
  rather than silently dropping the bound — a Slovak turn emitting `20.08.2026`
  must not quietly receive three weeks of notifications and call it yesterday.
  Twelve unit tests, most of them DB-backed.
- `notifications_search` registered unconditionally in `build_tools`, classified
  `ReadOnly` in `classify_tool` (which is what lets automations use it),
  dispatched through the rules gate, labelled in the notch activity strip.

Deliberate simplifications:

- No FTS5. Retention caps the table in the low thousands, so `LIKE` over the
  prefolded column is instant. Upgrade if a scan ever shows up in a trace.
- No `raw_json` catch-all column. It was in the original plan as insurance
  against a schema surprise, but the shape guard fails closed instead, and an
  unread column holding a second plaintext copy of every notification is a
  privacy cost with no reader.
- No rules entry. `RuleEngine::check` returns `Auto` for unregistered tools
  (`crates/rules/src/lib.rs:134`), so `notifications.search` is already `auto`
  and an explicit rule would be decorative.
- No taint tracking. Decision 6 is a formatting contract, not a mechanism —
  side-effecting tools already route through `request_tool_approval`, and
  notification text cannot call anything.

Also done, after Full Disk Access was granted:

- Reader: `collect()` copies Apple's db/wal/shm to a scratch dir, shape-guards
  the `record` and `app` tables, decodes the `data` bplist, converts
  CFAbsoluteTime to Unix. Verified live — 35 notifications, 34 with a delivery
  time, 2 with a thread id.
- `collect` → `apply` (ingest → advance watermark → purge → mark_synced), on a
  30s tick in `main.rs`, gated on the `enabled` flag. The file copy and plist
  decoding run in `spawn_blocking`, off the database lock chat turns need.
- Collector progress lives in `notifications_state.watermark`, deliberately
  apart from the mirror: "forget all" empties `notifications` and clears
  `last_sync_at` but keeps the watermark, so an erasure is not undone by the
  next poll and an emptied mirror can never pass itself off as current.
- `GET /notifications/status`, `POST /notifications/settings`,
  `POST /notifications/forget`. Switching the feed off wipes the mirror.
- Two opt-in live tests (`--ignored`) that read the real database, plus
  synthetic bplist fixtures so the decode logic tests anywhere.

Known gap: Apple's database carries only the bundle identifier, so `app_name`
is always NULL and an `app` filter matches identifiers like
`com.objective-see.dnd.helper`. The tool description says so. Resolving display
names would mean scanning app bundles, which is not worth it yet.

Remaining:

1. Swift settings toggle calling `POST /notifications/settings`, an FDA prompt
   with a deep link to System Settings, and a "forget all" button.
2. TCC attribution: whether a launchd-run `bagentd` can hold its own FDA grant,
   and whether the grant on `bagent.app` covers the daemon it spawns. Needs
   testing in both `make run` and `make bundle`.

Until 1 lands the feed stays off and the tool answers
`notifications_unavailable`. Flip it manually with:

```
curl -XPOST -H "Authorization: Bearer $(cat ~/Library/Application\ Support/bagent/daemon.token)" \
  -H 'content-type: application/json' -d '{"enabled":true}' \
  http://127.0.0.1:$(cat ~/Library/Application\ Support/bagent/daemon.port)/notifications/settings
```

## Why Full Disk Access is required

`group.com.apple.usernoted` is TCC-protected, in the same class as
`~/Library/Mail` and `~/Library/Messages`. The denial is at the TCC layer, so
even `ls` on the parent returns `Operation not permitted` — it is not a file-mode
or sandbox issue (verified: identical failure with tool sandboxing disabled,
while sibling group containers list fine). The grant must go to the responsible
app of whichever process reads it. Granting it resolved the block; the reader
now works.

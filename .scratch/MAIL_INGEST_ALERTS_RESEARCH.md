# Mail Ingest & Alert Notifications — Research Report and Implementation Plan

Date: 2026-02-14 (research pass against `main` @ `342be48`)
Status: Research + planning only. No code has been written.
Baseline inspected: `main` = `342be48` ("Merge pull request #37 … plan-bagent-notifications").

---

## A. Current architecture

### A.1 Apple Mail connector (`crates/connectors/apple_mail/src/lib.rs`, 1857 lines, single file)

What actually exists:

- **Discovery / metadata** — read-only SQLite access to Apple's Envelope Index
  (`~/Library/Mail/V10/MailData/Envelope Index`). Queries join
  `messages`/`subjects`/`addresses`/`mailboxes`/`recipients` **in Apple's DB**
  (not bagent's). Functions:
  - `list_inbox(limit, unread_only)` — newest-first metadata.
  - `search_messages(&MailSearchFilter)` — LIKE filters on sender/display/subject,
    inclusive date range (`date_from`, `date_to`), OR-keywords expansion.
    This is the only "FTS" that exists for mail: plain SQL LIKE over Envelope
    Index columns. There is **no bagent-side mail FTS5 table**.
  - `search_by_subject(query, limit)`.
  - `list_since(since_ts, limit)` — incremental by `date_received > since`,
    ordered ascending; this is the current incremental-sync primitive.
- **Body hydration** — `get_message(rowid)` prefers the local `.emlx` file
  (shard-path arithmetic tested in unit tests: `emlx_shard_calc_*`), parsed with
  `mailparse`; plaintext extracted via `extract_plain_text` / `strip_html`.
  Fallback: AppleScript automation against Mail.app (`hydrate_message` →
  `body_via_mail_app` → `run_automation_with_timeout`), with typed outcomes
  `MailBodyHydrationState { Readable, Empty, Unavailable, AutomationDenied,
  AutomationTimedOut, AutomationFailed }` and provenance `MailBodyOrigin {
  LocalEmlx, MailAutomation, Unavailable }`. Bodies are **not persisted**;
  hydration is on-demand.
- **Message-ID** — `MailMessage.message_id` is populated **only when the emlx
  file is parsed locally**. Metadata-only list/search queries return
  `message_id: None`. Envelope Index does not expose a usable RFC Message-ID
  column through the current code.
- **Attachments** — metadata from MIME tree (`MailAttachment`, part_index),
  bytes via `get_message_attachment[_base64]`; cached metadata in
  `mail_attachments` (V9).
- **Open** — `open_message(message_id, subject, sender)` opens the message in
  Mail.app via AppleScript. Used by `/mail/open` and the Swift `openMail`
  click-through (`ChatViewModel.openMail`, line ~2073).
- **Language** — whatlang detection (`detect_language`) per message.
- **AppleScript search fallback** — `search_messages_via_applescript` exists as
  a secondary path when the Envelope Index is unreadable (TCC).

### A.2 Daemon-side mail sync (`crates/daemon/src/main.rs`)

- Local cache table `mail_cache` (V3): keyed by Envelope Index ROWID, stores
  subject/sender/sender_display/received_at/is_read/mailbox_url/language.
  **No body, no thread grouping, no provider identity, no FTS.**
- `mail_sync_inner(db, mail, _memory)`:
  reads `connectors.last_sync_at` where `kind='apple_mail'`, first sync fetches
  up to 5000 rows, otherwise 500; `INSERT OR REPLACE INTO mail_cache`;
  updates `last_sync_at`. Note: the watermark is *sync time*, not
  `date_received`, so clock-skewed late arrivals can be missed (relevant to the
  24 h briefing design — see H).
- Two triggers today:
  - Battery-aware poller: startup sync after 10 s if on AC, then every 300 s,
    skipped entirely on battery power.
  - FSEvents watcher (notify crate) on `Envelope Index-wal`, debounced 2 s,
    AC-gated.
- HTTP: `POST /mail/sync`, `GET /mail/inbox`, `GET /mail/message/:rowid`,
  `POST /mail/open`, attachment endpoints, `POST /mail/cache/clear`
  (purges cache rows older than 30 days).

### A.3 Agent tools

Registered per turn in the daemon chat handler; actual mail tool surface is:

| Tool | Exists | Notes |
|---|---|---|
| `mail_search` | yes | sender normalization retry (tokenize sender into AND keywords on empty result); emits `mail_found` SSE |
| `mail_list_inbox` | yes | |
| `mail_read` | yes | hydration states surfaced as corrective text; automation-denied states distinguish unattended runs |
| `mail_open` | yes | approval-free? currently read-classified; opens Mail.app |

`docs/CONNECTORS.md` additionally lists `mail_get_thread`,
`mail_create_draft`, `mail_send_draft`, and an FTS5-indexed `mail_search` —
**none of these exist in code** (see §C).

Tool loop (`agent_exec::run_agent_loop`, shared by chat and automations):
max 5 rounds, 8 tool calls per turn, rules-engine gate (`auto`/`ask`/
`forbidden`), audit row per call, model output never authorizes anything.

### A.4 Evidence orchestrator (exists on main)

`agent_exec.rs` already routes certain turns deterministically:
`prepare_turn_routing` → `execute_evidence_turn` with
`desired_mail_read_count(user_message)` deterministic prefetch for
"summarize my emails" style turns, evidence validation events
(`evidence_validation`, `evidence_polish`, `evidence_outcome` SSE types),
and Stage-8 synthesis service (synthesis client = BaseRT, swappable for
acceptance fixtures). The `.scratch/reliable-grounded-mail-web/map.md`
wayfinder record documents the measured rationale: deterministic evidence
acquisition around a lazy-warm larger synthesis model with one bounded small-
model fallback. **This is the exact pattern the daily briefing and alert
classifier should reuse** rather than raw prompt-and-hope tool loops.

### A.5 Conversational reference resolution (validated-candidate precedent)

Migrations V16/V17 + `crates/daemon/src/reference_resolution/`: sealed typed
mentions, opaque anchors, HMAC bindings, XChaCha ciphertext for display values,
`query_authorizations` checked immediately before any external provider
operation, `dynamic_candidate_bindings_v17` — i.e., the repository already has
a production pattern for **"execution code discovers and validates candidates;
the model may only select opaque IDs."** The alert action design (§J) should
follow this philosophy without importing its full cryptographic machinery in v1.

### A.6 Notifications mirror (PR #37, merged into main)

- Migration `V18__notifications.sql`: `notifications` (unique `source_id`,
  folded `search_text`/`app_text` for Slovak), `notifications_state` watermark
  separate from the mirror (so "forget all" doesn't rewind ingestion), seed
  `connectors` row `kind='notifications', enabled=0` (**off by default**).
- `crates/daemon/src/notifications.rs`: ingest (denylist at write), purge
  (30-day rolling), search with thread collapsing `(app_bundle_id, thread_id)`,
  fail-closed schema guard, untrusted-text rendering. 30 s poll in the daemon,
  independent of the app (daemon is a launchd agent).
- Tool `notifications_search`, rules action default `auto` (read-only);
  **tool-only injection — notification text never auto-enters prompts**.
- `docs/NOTIFICATIONS_PLAN.md` says the Swift settings toggle was still
  pending at merge time.

### A.7 Notch UX state that already exists

All in `apps/macos/Sources/bagent/` on main (no NotchProjection files on main):

- **CmuxNotificationController.swift** — `CmuxEventMonitor` spawns
  `cmux events --reconnect`, parses NDJSON, classifies attention vs finished
  hooks, dedupes by `sessionId ?? workspaceId`, resolves workspace names,
  supports focus routing back into cmux.
- **ChatViewModel.swift** (~2142 lines):
  - `cmuxPending: [CmuxNotification]` (max 10, newest first, replace-by-dedupeKey)
  - `cmuxBanner: CmuxNotification?` transient banner
  - `cmuxDeparting: [CmuxDeparture]` fly-off animation tokens
  - `cmuxDotKind` — attention (amber) outranks finished (green)
  - `markAllCmuxSeen()`, `markCmuxSeen(workspaceId:)`, `focusCmux(_:)`
    click-through, `beginCmuxDeparture()` guarded by
    `accessibilityDisplayShouldReduceMotion`.
  - Separately: `pendingConnectorActions` left-wing chips (mail/odoo/file),
    latest-per-kind upsert, cleared on next user message.
- **NotchWindowController.swift**: banner shows only while
  `notchInteractionMode == .collapsed`, auto-dismisses after 5 s
  (`DispatchWorkItem`, latest wins); hover reveals count badge ("+N");
  pending approvals preempt everything else (UI_DESIGN.md lines 77–81: a
  pending approval beats all modes; gated writes auto-deny after 60 s).
- **ChatView.swift**: corner dot layer, banner layer, hover-reveal logic,
  reduced-motion-aware transitions.

There is **no persistent alert store in Swift** — cmux alerts are ephemeral
(in-memory, lost on relaunch). That is fine for cmux but wrong for operational
alerts; hence daemon-authoritative alerts (§K).

### A.8 Automations / scheduler

- `crates/automations/` — pure typed schedule semantics (once / every-N /
  daily / weekdays / selected / weekly, IANA zone DST-correct, catch-up window,
  missed-run decisions).
- `crates/daemon/src/scheduler.rs` — daemon-owned loop
  (`run_scheduler`, `scheduler_step`, `recover_on_startup`), woken early via
  `automations_changed: Arc<Notify>`, bounded run slots
  (`run_slots: Arc<Semaphore>` ≈ 2 concurrent), atomic overlap claims.
- `crates/daemon/src/agent_exec.rs` — automations execute through the **same**
  `run_agent_loop` as chat with `ExecOrigin::unattended()`: side-effecting
  tools require fresh approvals (with automation provenance in
  `pending_approvals.origin_json`, V14); read-only tools run unattended.
- Results surface via `automations_api.rs` + daemon-wide broadcast
  `GET /events` (`events_tx`, fire-and-forget, redacted payloads; clients
  refetch authoritative records). Swift shows run status/output in the notch
  automations view.

### A.9 Model runtime authority

On **main**, BaseRT is an external service managed by the Swift app as a
dedicated LaunchAgent on `127.0.0.1:8082`. The daemon holds one
`BaseRtClient` (`state.inference`) used by everything (chat stream, evidence
synthesis, classifiers). There is no admission/idle management on main —
the worktree branch adds exactly that.

### A.10 Legacy generic `messages` table

`V2__full_schema.sql` creates a minimal `messages` table (id/source/external_
id/language/subject/body/sender/received_at/indexed_at). **No daemon or
connector code reads or writes it** (all `FROM messages m` occurrences are
against Apple's Envelope Index inside the apple_mail connector). It is dead
schema, but it occupies the name `messages`.

---

## B. Worktree state

`git worktree list` (2026-02-14):

| worktree / branch | HEAD | state vs main | relevant work | implications |
|---|---|---|---|---|
| `/Users/oliver/Programming/bagent` (main) | `342be48` | — | notifications mirror merged (PR #37), browser merged (PR #35), reference resolution V16/V17, evidence orchestrator | implementation baseline |
| `t3code/basert-notch-automation-ux` → `~/.t3/worktrees/bagent/t3code-183f19bf` | `d57e574` (ahead 3 of origin) | **18 ahead / 36 behind main** | Work Coordinator (`work_coordinator.rs`, `unified_work.rs`, `cutover.rs`), Model Runtime (`model_runtime.rs`), Current Chat (`current_chat.rs`), automation sessions, `NotchProjection.swift`, `NotchEventConsumer.swift`, `/work/snapshot` + `/work/events` cursor stream with consumer fencing and gap recovery, UI relaunch protocol | **migration collision V15–V18 both sides**; branch extends to V23. Must be reconciled before any new migration lands (§M) |
| `t3code/add-embedded-safari-browser` | `86a07d7` | fully merged (via ce57a19, PR #35) | bagent Browser | no action; branch can be deleted |
| `t3code/plan-bagent-notifications` | `7b72205` | fully merged (PR #37) | notifications mirror | no action; branch can be deleted |
| `t3code/design-conversational-reference-resolver` | `342be48` | == main | planning worktree only | no divergence |
| `t3code/ollama-macbook-models` | `1ec0072` | merge-base == its HEAD ⇒ fully contained in main | historical | no action |
| `t3code/plan-mail-ingest-alerts` → `~/.t3/worktrees/bagent/t3code-ee2c934a` | `342be48` | == main, fresh | presumably created for this feature | implement here (or fresh branch cut from post-reconciliation main) |

Also present: a stash on main ("resolver transfer safety checkpoint") and
untracked files including `BagentDataDirectory.swift` and
`.scratch/reliable-grounded-mail-web/` (resolved wayfinder spec for grounded
mail/web flows — directly reusable context, see A.4).

### Reconciliation verdict

`basert-notch-automation-ux` must rebase onto current main **first**. Its
migrations V15–V23 collide with main's V15–V18 (different content entirely):

| version | main | branch |
|---|---|---|
| V15 | automation_reference_blocked | work_coordinator_foundations |
| V16 | conversational_reference_resolution | unified_work_cutover |
| V17 | reconcile_provider_authorization | work_activity_projection |
| V18 | notifications | notch_projection_indexes |
| V19–V23 | — | automation_session_contract … stage8_canonical_cleanup |

Because refinery orders strictly by version number, the branch's migrations
must be renumbered to start after main's highest (post-rebase: rename its
V15→V19 … V23→V27, updating the `include_str!` paths in `work_coordinator.rs`)
or the branch rebases and main's future work starts after V23. Pick **one**
canonical sequence before this feature's first migration. Recommendation:
rebase branch onto main and renumber branch migrations V15–V23 → V19–V27;
new feature migrations then begin at V28. If the branch will land long before
this feature, invert: let the branch keep V15–V23 (it becomes main) and start
feature migrations at V24. Either way: **decide once, document in the PR.**

---

## C. Important discrepancies (docs vs code)

1. **DATA_MODEL.md `messages` table** — describes UUID-v7 PK, recipients JSON,
   body_html, thread_id, mailbox, is_read, UNIQUE(source, external_id),
   `messages_fts` FTS5, hybrid vector+BM25 retrieval. Reality (V2): integer-PK
   minimal table, unused by any code; no messages FTS anywhere; embeddings
   table exists (V5) with no sqlite-vec runtime wired for messages.
2. **CONNECTORS.md mail tool list** — lists `mail_get_thread`,
   `mail_create_draft`, `mail_send_draft`, FTS5-backed `mail_search`, and
   claims "read inbox only". Reality: four tools (`mail_search`,
   `mail_list_inbox`, `mail_read`, `mail_open`); LIKE-based search across all
   non-deleted mailboxes; no draft/send path exists.
3. **"Incremental sync"** — `list_since` uses wall-clock `last_sync_at`
   compared against `date_received`; messages delivered with skewed timestamps
   during downtime fall outside the watermark. Fine for a cache; unsafe as the
   sole basis of a 24 h report guarantee (see H).
4. **NOTIFICATIONS_PLAN.md** — accurate; only the Swift settings toggle lagged
   the doc.
5. CLAUDE.md architecture table matches reality.

Rule applied: code/migrations win; docs above need correction as part of
phase work.

---

## D. Reusable infrastructure (do not rebuild)

| Need | Already exists | Where |
|---|---|---|
| Scheduled execution | typed schedules + daemon scheduler loop, catch-up, slots | `crates/automations`, `scheduler.rs` |
| Unattended AI execution with safety | `run_agent_loop` + `ExecOrigin::unattended` + fresh-approval rule | `agent_exec.rs` |
| Deterministic evidence acquisition + synthesis w/ fallback | evidence orchestrator, synthesis service, acceptance-fixture seam | `agent_exec.rs`, `daemon/src/evidence/` |
| Validated-candidate-before-model pattern | reference_resolution query authorizations + candidate bindings | `reference_resolution/`, V16/V17 |
| Untrusted third-party text handling + watermark + denylist + fail-closed feed | notifications collector | `notifications.rs`, V18 |
| Daemon→Swift eventing | broadcast `GET /events`, publish helpers | `main.rs` (`AppState::publish_event`) |
| Notch ambient alert UX | banner/dot/departure/dedupe/click-through/reduced-motion | `CmuxNotificationController.swift`, `ChatViewModel.swift`, `NotchWindowController.swift`, `ChatView.swift` |
| Apple Mail read/hydrate/open | MailConnector incl. emlx + AppleScript fallback + TCC patterns | `apple_mail/src/lib.rs` |
| Secrets storage | KeychainStore (Swift); Keychain custody precedent in Rust exists in reference_resolution crypto module | `KeychainStore.swift`, `daemon/src/reference_resolution/` |
| Rules engine gating | `auto`/`ask`/`forbidden` per action | `crates/rules` |
| Audit | append-only hashed `audit_entries` | V1 |

Gaps: no Gmail connector; no canonical cross-provider mail store; no alert
persistence/event type; no validated URL-action registry; no general AlertCenter.

---

## E. Proposed target architecture

```
                       ┌────────────────────────────────────────────┐
                       │                bagentd (launchd)           │
                       │                                            │
 Apple Mail            │  ┌──────────────────┐   ┌───────────────┐  │
 Envelope Index ──watch/poll──▶ AppleMailSource │   │ GmailSource   │  │
 .emlx bodies          │  │  (existing conn) │   │ (new conn)    │  │
                       │  └────────┬─────────┘   └──────┬────────┘  │
                       │           ▼                    ▼           │
                       │      ┌──────────────────────────┐          │
                       │      │   MailIngestCoordinator  │          │
                       │      │  dedupe · hydrate policy │          │
                       │      └────┬───────────────┬─────┘          │
                       │           ▼               ▼                │
                       │  ┌──────────────┐  ┌───────────────────┐   │
                       │  │mail_messages │  │ mail_alerts       │   │
                       │  │+ source_refs │  │ (derived, persisted│  │
                       │  │+ sync_state  │  │  projection)      │   │
                       │  └──────┬───────┘  └────────┬──────────┘   │
                       │         │                   │              │
                       │  ┌──────▼────────┐   ┌──────▼───────────┐  │
                       │  │ Briefing job  │   │ AlertDetector    │  │
                       │  │ (automation,  │   │ prefilter→links→ │  │
                       │  │ existing      │   │ validate→BaseRT  │  │
                       │  │  scheduler)   │   │ json→AlertAction │  │
                       │  └──────┬────────┘   └──────┬───────────┘  │
                       │         │                   │              │
                       │         ▼                   ▼ publish      │
                       │        GET /events  ◀───────┘ (alert.new)  │
                       └────────────┬───────────────────────────────┘
                                    │ SSE / REST (presentation only)
                       ┌────────────▼───────────────┐
                       │ Swift notch: AlertCenter   │
                       │ (generalized cmux pill/    │
                       │  banner/dot; approvals     │
                       │  still preempt)            │
                       └────────────────────────────┘
```

Principles: daemon owns truth and execution; Swift projects. One mail domain,
one alert pipeline, one scheduler, one model client. Alerts are a derived
projection of mail — never a special message type.

---

## F. Gmail design

Verified against primary Google documentation (developers.google.com,
/workspace/gmail/api/auth/scopes, .../identity/protocols/oauth2/native-app,
.../gmail/api/reference/rest/v1/users.history/list — fetched 2026-02-14):

### OAuth
- Flow: **Authorization Code + PKCE for installed apps**; verifier ≥43 chars,
  S256 challenge recommended. Redirect: **loopback IP**
  (`http://127.0.0.1:<ephemeral port>`) — Google documents deprecation of
  loopback **on mobile apps only**; desktop installed-app use remains the
  documented approach. (Custom-scheme is the alternative.)
- Scope: **`https://www.googleapis.com/auth/gmail.readonly`** — minimum scope
  that reads bodies and settings; also accepted by `users.history.list`
  (verified: history.list requires one of `mail.google.com`, `gmail.modify`,
  `gmail.readonly`, `gmail.metadata`). No write scope requested — bagent never
  sends via Gmail in this feature.
- ⚠️ **Restricted scope reality**: `gmail.readonly` is a *restricted* scope.
  Google's policy requires restricted-scope verification plus a CASA security
  assessment **if you distribute the app**. For a local-first personal app the
  practical pattern is: user creates their own Google Cloud project, configures
  OAuth client (Desktop type), adds themselves as test user. Caveat: apps in
  "Testing" status get refresh tokens that expire in 7 days — acceptable v1
  trade-off (bagent surfaces "reconnect account"), with a documented path to
  verification later. This is a product decision to confirm (§O).
- Tokens: refresh token in **Keychain**. Custody choice: the daemon must be
  able to refresh tokens while the app is closed (launchd agent). Reference
  resolution already establishes Keychain custody inside `bagentd` (Rust), so
  store via the same mechanism (security-framework/keyring crate); Swift
  settings UI reads status via daemon API, never touches the secret.
  Client ID/secret: Desktop-type client secret is treated as non-confidential
  by Google for installed apps; still store in Keychain, configurable in
  Settings (per-user project).

### Initial sync
- `GET /gmail/v1/users/me/messages` with `labelIds=INBOX` (+ optional
  `q=newer_than:1d` for bootstrap scoping), `maxResults=500` pagination via
  `pageToken`. Each item yields `id` + `threadId` + `historyId` on response.
- Fetch each message `users.messages.get?format=raw` (full RFC 822 → parse
  locally with the existing `mailparse` stack, giving identical body/
  attachment/Message-ID extraction semantics to the emlx path) or
  `format=full` for structured parts. Prefer `raw` for parser parity.
- RFC Message-ID comes from the `Message-ID` header — always available here,
  unlike Apple Mail metadata queries.

### Incremental sync
- **Chosen: periodic `users.history.list` polling with `startHistoryId`**
  (historyTypes `messageAdded`, filtered to INBOX label additions).
  - Verified behavior: historyId valid "typically at least a week", may be
    hours in rare cases; invalid/expired `startHistoryId` → HTTP 404 →
    perform full resync (bounded by `q/newer_than:` window). No
    `nextPageToken` → store returned `historyId` as new cursor.
  - Poll cadence: piggyback on the existing mail poller (300 s AC-gated;
    on-demand when user asks about mail; immediate catch-up on wake).
- **Rejected: `users.watch` + Pub/Sub push** — requires a cloud project with
  Pub/Sub topic + renewing watch expiries + an endpoint or polling anyway;
  violates local-first for marginal gain. Revisit only if latency budgets for
  alerts demand it (v1 alert latency of ~minutes is acceptable; Slack/Sentry
  alerts could later come via Notification Center mirror as secondary signal).
- Dedupe of overlapping history events: idempotent upsert on Gmail `messageId`;
  duplicate `messageAdded` records collapse naturally.
- Rate limits: per-user quotas are generous for one account; implement 429/
  exponential backoff with jitter, honor `Retry-After`; batch gets
  (`/batch` up to 100 ops) for initial hydration if needed.

### Account lifecycle
- `mail_accounts` row per Google account (email address as stable key),
  enabled flag, last error surfaced in Settings → Connectors (same place the
  notifications toggle landed), sync health (cursor age, last success/failure).

---

## G. Canonical mail model

Extend, don't replace. Keep `mail_cache` (it feeds existing chat-context paths)
but make it an Apple-Mail-specific detail; introduce canonical tables. The V2
`messages` table is dead code — **drop it in the same migration** so the good
name isn't squatting (refinery has no drop helper needed; plain DDL works).

```sql
-- V-next: canonical mail domain
CREATE TABLE mail_accounts (
    id            TEXT PRIMARY KEY,          -- uuid v7
    provider      TEXT NOT NULL CHECK(provider IN ('apple_mail','gmail')),
    external_key  TEXT NOT NULL,             -- apple_mail: 'local'; gmail: account email
    display_name  TEXT,
    enabled       INTEGER NOT NULL DEFAULT 1,
    status        TEXT NOT NULL DEFAULT 'ok',-- ok|auth_required|error|disabled
    last_error    TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    UNIQUE(provider, external_key)
);

CREATE TABLE mail_messages (
    id            TEXT PRIMARY KEY,          -- uuid v7, canonical identity
    message_id    TEXT,                      -- normalized RFC Message-ID (<>, lowercased); NULL if unknown
    subject       TEXT NOT NULL DEFAULT '',
    sender        TEXT NOT NULL DEFAULT '',
    sender_display TEXT,
    recipients    TEXT,                      -- JSON array (To/Cc)
    received_at   INTEGER NOT NULL,          -- unix secs, Date header normalized
    language      TEXT,
    mailbox_folder TEXT NOT NULL DEFAULT '', -- 'INBOX', 'Sent', gmail labels set
    is_read       INTEGER NOT NULL DEFAULT 0,
    body_state    TEXT NOT NULL DEFAULT 'metadata',
                  -- metadata | hydrated | unavailable | needs_automation
    body_text     TEXT,                      -- stripped plaintext, NULL until hydrated
    thread_key    TEXT                       -- normalized References/In-Reply-To root or gmail threadId
);
CREATE INDEX mail_messages_received ON mail_messages(received_at DESC);
CREATE INDEX mail_messages_msgid    ON mail_messages(message_id) WHERE message_id IS NOT NULL;
CREATE INDEX mail_messages_window   ON mail_messages(mailbox_folder, received_at DESC);

CREATE TABLE mail_source_refs (
    message_rowid INTEGER NOT NULL REFERENCES mail_messages(rowid) ON DELETE CASCADE,
    account_id    TEXT NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
    provider_uid  TEXT NOT NULL,             -- apple: Envelope ROWID; gmail: messageId
    extra_json    TEXT,                      -- gmail threadId, labels; apple mailbox_url
    last_seen_at  INTEGER NOT NULL,
    PRIMARY KEY (account_id, provider_uid)
);

CREATE TABLE mail_sync_state (
    account_id    TEXT PRIMARY KEY REFERENCES mail_accounts(id) ON DELETE CASCADE,
    cursor        TEXT,                      -- gmail historyId / apple max(date_received)
    last_success  INTEGER,
    last_attempt  INTEGER,
    backoff_until INTEGER
);
```

Notes:
- Dedupe order: (1) exact normalized RFC `message_id` match → same canonical
  row, add second source ref; (2) else distinct row. No fuzzy merging in v1
  (subject/sender/date heuristics stay out — conservative requirement).
- Apple Mail metadata queries cannot supply Message-ID; therefore Apple-only
  rows have `message_id NULL` until a body hydration parses it. When the Gmail
  source sees the same Message-ID later, the Apple row gains a second ref
  (retro-link) rather than a duplicate. Messages lacking Message-ID remain
  intentionally distinct per provider.
- Optional later: `mail_links` (extracted candidate URLs per message) — see I/J;
  v1 can compute links transiently during alert detection, persisting only
  validated candidates inside `alert_actions`.

---

## H. Daily briefing design

**Answer: ingestion is continuous/incremental; only synthesis is scheduled.**
The existing automation system runs the briefing; it must not call connectors
from the prompt. New capability: an internal (user-invisible or listed) task
kind whose execution calls a deterministic briefing builder instead of a free
tool loop.

Data flow:

1. Rolling window computed deterministically: `[now − 24h, now]` (unix secs).
2. Query canonical store:
   `WHERE received_at >= :start AND received_at < :end AND mailbox_folder NOT IN ('Sent','Drafts','Trash','Junk')`
   (Apple mailbox_url mapping + Gmail label exclusion). Cap N=400 headers;
   page through by `received_at DESC` batches of 50.
3. Hydration policy: hydrate bodies lazily — first pass over headers +
   prefilter (sender-domain heuristics: known alert senders, calendar, direct
   mail to the user vs bulk); hydrate top-K (e.g., 80) via existing
   `get_message`/emlx path (Apple) and stored `body_text` (Gmail hydrated at
   ingest for small messages, deferred for large ones).
4. Thread grouping by `thread_key` (fallback: normalized subject + sender).
   Collapse duplicates via source refs (`duplicates_collapsed` count).
5. Classification + synthesis: two-stage, mirroring the evidence orchestrator's
   measured policy (`.scratch/reliable-grounded-mail-web/map.md`): cheap
   per-chunk triage on Qwen3-4B (structured JSON buckets), final synthesis on
   the preferred synthesis model (35B-A3B per wayfinder measurement), one
   bounded repair round, deterministic fallback = grouped listing sorted by
   heuristic priority if either stage fails.
6. Output: fixed Slovak-language structure consistent with notch conventions:
   Needs action / Important / Waiting on you / Automated alerts / FYI /
   Noise skipped (count only), plus Sources block (per-account counts,
   duplicates collapsed, source health/errors).
7. Delivery: result persists as an automation run (existing machinery) AND
   emits `briefing.ready` on `/events` → AlertCenter card + notch chip;
   clicking opens the notch expanded report (markdown renderer already exists:
   `NotchMarkdown.swift`). Works with notch closed because everything is
   daemon-side; the app refetches on next attach.

Guarantees note: fix `mail_sync_inner`'s time-skew hole for the canonical
store by using `max(received_at)` as the Apple cursor (like Gmail historyId),
not wall-clock sync time.

---

## I. Alert detection architecture

Pipeline (runs inside the daemon, triggered from ingest when a new canonical
message arrives; async, off the ingest transaction):

```
new mail_messages row
  → prefilter (deterministic, zero cost): sender domain/address in
    {sentry.io, slack.com, vercel.com} + subject patterns
  → skip unless candidate
  → hydrate body (already stored for Gmail; emlx for Apple)
  → extract links (reuse strip_html-adjacent parsing; collect hrefs)
  → normalize + classify each link against provider validators:
      sentry: https://<org>.sentry.io/issues/<id>/… or /organizations/<org>/issues/<id>
      vercel: https://vercel.com/<team>/<project>/… deployments
      slack:  https://<workspace>.slack.com/archives/<channel>/p<ts>
      reject tracking/unsubscribe/redirectors (explicit deny patterns)
  → persist candidates (opaque ids) [alert_action_candidates]
  → BaseRT generate_json (Qwen3-4B, classifier budget): given subject +
    trimmed body + CANDIDATE LIST ONLY → { kind, provider, severity, title,
    summary, action_candidate_id? }
  → if severity >= threshold: persist alert + chosen action (copy URL from the
    validated candidate row, never from model text)
  → publish_event({"type":"alert.new", id, …redacted}) ; audit entry
```

Provider adapters: trait `AlertProvider` (match_sender, validate_link,
expected_path_regex, display_name) — adding GitHub/Stripe/Linear/Cloudflare =
new table-free adapter + fixture tests.

Dedup/coalescing: `dedupe_key = hash(provider, entity-id from validated link
path or thread)`; re-alert within cooldown window updates existing alert
(severity escalation re-notifies), otherwise silent bump.

Prompt-injection stance: email body is data, wrapped/labeled like
notifications_search results; the classifier prompt instructs selection from
candidate ids only; the executor ignores any URL appearing in model output —
structural enforcement, not prompt wording.

---

## J. Validated action security model

Reuse the reference-resolution principle: *execution code discovers and
validates; the model selects opaque references* (see
`crates/daemon/src/reference_resolution/query.rs` candidate bindings;
ADR-0001 deterministic grounding).

Concrete guarantees:

1. Links are extracted by daemon code from the parsed email, never from model
   output.
2. Each link passes scheme (`https` only), host allowlist (exact suffix match
   per provider, e.g. `*.sentry.io`), and path-shape regex before becoming a
   candidate. Unsubscribe/tracking/marketing links are classified separately
   and are never eligible actions.
3. Candidates live in `alert_action_candidates` with opaque UUIDs; the LLM
   receives only `(candidate_id, provider, link_kind, display_label)`.
4. `AlertAction.action_url` is materialized **by copying from the candidate
   row** upon model selection. Any `action_candidate_id` not found → no action
   (fail closed). Malformed JSON → deterministic fallback (no action, title
   from sender).
5. Opening: default system browser via `NSWorkspace.open` for Sentry/Vercel/
   Slack web URLs (they're authenticated web destinations; bagent Browser adds
   no value and shares the dedicated profile unnecessarily). Native-app
   handoff stays out of v1. Rationale aligns with browser ADRs: Browser
   Sessions exist for agent-driven browsing, not human click-through.
6. Every alert action creation and open is audited (`audit_entries`).

This mirrors how `mail_open` already only opens references resolved by
connector code, extended with third-party-content hostility.

---

## K. Notch Alert Center design

Generalize the existing cmux pattern rather than adding a parallel system.
Introduce a Swift `AlertCenter` (new file, consumed by ChatViewModel) whose
items unify sources behind one shape:

```swift
struct AlertItem: Identifiable {
    let id: UUID                 // daemon alert id
    let source: Source           // .mail(.sentry|.slack|.vercel), .cmux(CmuxNotification), .briefing
    let kind: String             // incident | mention | finished | briefing …
    let severity: Severity       // critical | warning | info   (attention/green map onto this)
    let title: String
    let contextLines: [String]
    let occurredAt: Date
    let dedupeKey: String
    var seen: Bool
    let action: AlertAction?     // .url(URL) | .openMail(MailRef) | .focusCmux(...)
}
```

Behavior (all lifted from existing code paths):

- Collapsed: single corner dot (severity-priority coloring) + count — same
  layer as today's `cmuxDotLayer`.
- Fresh alert while collapsed: transient banner 5 s, latest wins, cancel-on-
  newer (`NotchWindowController.onCmuxNotification` pattern, generalized to
  `onAlert`).
- Hover: reveal latest title + "+N".
- Click: execute only the validated action; mark seen; departure animation
  (Reduce-Motion guarded).
- Expanded notch (input mode): alerts render as a section above input, never
  stealing mode from pending approvals — approvals preempt (UI_DESIGN rule).
- State authority: daemon persists alerts (table + seen flag API
  `POST /alerts/:id/seen`); Swift is presentation + ephemeral animation state.
  Relaunch-safe (unlike cmuxPending which stays ephemeral/in-memory by nature
  of being a live-process cue — cmux items keep their current lifecycle and
  simply become another `AlertSource` feeding the same views).
- Geometry ceiling: cap rendered items (dot count, banner) exactly as cmux
  does (max 10 pending, 1 banner).

Priority order in collapsed notch: pending approval > alert banner > connector
chips > idle.

---

## L. Work Coordinator / NotchProjection integration

Branch `t3code/basert-notch-automation-ux` introduces daemon-authoritative
Works, `/work/events` cursor stream with consumer fencing + gap recovery,
NotchProjection presentation states, and ModelRuntime (shared idle timeout,
single BaseRT authority).

- **Should an alert be a Work? No.** Works model admitted, cancellable
  executions with approval gates and terminal lifecycle
  (`NotchWorkState` queued→running→waitingForApproval→completed…). An alert is
  a durable observation/projection, not an execution; forcing it into `works`
  would abuse terminal-state semantics and attention_state. However, *alert
  classification* (BaseRT call triggered by ingest) is background execution:
  under the unified coordinator it should be admitted as lightweight
  non-interactive work (fairness queue, capacity slots) so it cannot starve
  foreground chat or steal model slots; on main today it simply reuses
  `state.inference` with a semaphore — no second runtime authority either way.
- **Where alert state lives:** daemon tables + `/events` (and later a
  projection feed entry). Swift holds only seen-animation state. On the
  worktree branch, add alerts as a new event category in the snapshot/events
  contract (a small extension to `NotchWorkSnapshot`/event enum) — do **not**
  create a second Swift event stream.
- **How alerts affect NotchProjection:** as an additive presentation input:
  snapshot gains `unread_alert_count` + latest alert summary; events gain
  `alert_new` / `alert_updated`. Work lifecycle untouched. If the branch lands
  first, phases 6–7 target the projection consumer instead of
  ChatViewModel-direct wiring; if main stays authoritative, the AlertCenter
  abstraction isolates the difference behind one consumer type (deliberately
  shaped like `NotchEventTransport`).

---

## M. Migration / branch reconciliation plan

Prerequisites, in order:

1. Land or clearly park `t3code/basert-notch-automation-ux`: rebase onto main,
   resolve code conflicts in `main.rs`/`agent_exec.rs` (36 commits apart),
   **renumber its V15–V23 → V19–V27** (update `SCHEMA` include paths +
   `SCHEMA_VERSION` in `work_coordinator.rs`), verify refinery upgrade path on
   a copy of a real dev DB. Merge. (If parking instead: document that main's
   next migration is V19 and the branch will rebase later — but do not start
   feature migrations until decided.)
2. Delete stale fully-merged branches/worktrees (safari-browser,
   plan-bagent-notifications, ollama-macbook-models) to reduce noise.
3. Drop legacy V2 `messages` table in the first feature migration (after
   confirming zero readers — verified in §A.10).
4. Feature migrations then take the next free numbers (V24… under "branch
   keeps V15–V23", or V28… under "renumber branch"). **No feature migration is
   written until step 1 concludes.**

---

## N. Implementation phases

Phase 1 — Canonical mail store + Apple ingestion
- Goal: single canonical store fed by Apple Mail; dedupe within provider.
- Files: `migrations/V-next__canonical_mail.sql`; `crates/daemon/src/mail_store.rs` (new);
  `main.rs` (rewrite `mail_sync_inner` to dual-write canonical + cache;
  cursor = max(received_at)); `crates/connectors/apple_mail` unchanged except exposing message_id when parsed.
- DB: tables in §G minus gmail specifics. API: none new yet.
- Tests: ingest fixtures (rowid stability, re-sync idempotence, cursor skew
  case, folder mapping, 24h-window query boundaries).
- Acceptance: existing chat flows unaffected (`mail_cache` intact); canonical
  rows populate; `cargo test -p bagent-daemon` green.
- Deps: migration reconciliation (§M).

Phase 2 — Daily briefing (Apple-only)
- Goal: scheduled 24h report from canonical store.
- Files: `automations_api.rs`/`scheduler.rs` (internal task kind),
  `daemon/src/briefing.rs` (new), Swift: chip + markdown report view reuse.
- Tests: empty inbox, model failure → deterministic fallback, batching,
  window math DST-boundary, duplicates collapsed count.
- Acceptance: briefing appears as automation run + `/events` ping; notch
  shows result; unattended run performs zero side-effecting tools.
- Deps: Phase 1.

Phase 3 — Gmail connector
- Goal: OAuth + initial + incremental sync into canonical store.
- Files: `crates/connectors/gmail/` (new), `mail_store.rs` (source refs,
  retro-link dedupe), Settings UI (account connect/disconnect/status),
  Keychain token custody in daemon.
- Tests (offline-first with recorded HTTP fixtures): PKCE/refresh flow,
  pagination, MIME raw parsing parity with emlx path, history incremental,
  expired cursor → resync, 429 backoff, duplicate history events.
- Acceptance: real-account smoke test; same message via Apple+Gmail collapses
  to one canonical row with two refs.
- Deps: Phase 1; Google Cloud project setup (user-side).

Phase 4 — Cross-provider briefing
- Goal: briefing over both providers, source-health reporting.
- Tests: both providers healthy, one down, partial body availability, large
  inbox batching, no mail.
- Deps: Phases 2–3.

Phase 5 — Alert detection + validated actions
- Goal: Slack/Sentry/Vercel alerts persisted + published.
- Files: `daemon/src/alerts.rs` (pipeline + provider adapters), migrations
  (`alerts`, `alert_action_candidates`, `alert_actions`), `/events` type,
  audit entries.
- Tests: vendor fixtures (real sample emails), marketing-email negative,
  severity classification, dedupe/coalescing, injection attempts, arbitrary
  URL rejection, unsubscribe links, malformed HTML, no-valid-link case.
- Acceptance: end-to-end alert from ingested fixture; model never influences
  URL (unit-proven by construction tests).
- Deps: Phase 3 (Gmail) strongly recommended; works with Apple-only too.

Phase 6 — Notch Alert Center UX
- Goal: generalized pill/banner/dot/click-through.
- Files: `AlertCenter.swift` (new), `ChatViewModel.swift`, `ChatView.swift`,
  `NotchWindowController.swift`, Settings toggles.
- Tests: banner timing, unseen count, dedupe, seen persistence across
  relaunch (daemon restart), click-through executes validated action only,
  approval preemption, Reduce Motion, geometry ceiling.
- Deps: Phase 5 (cmux integration can land earlier independently).

Phase 7 — Unified-work alignment
- Goal: fold into NotchProjection/event-stream contract once
  `basert-notch-automation-ux` merges; classification admitted through Work
  Coordinator fairness queue; alerts added to snapshot/events categories.
- Deps: §M reconciliation complete.

Recommended sequencing rationale: each phase ships value alone and keeps the
privacy model (nothing leaves device except Gmail API reads the user opted
into).

---

## O. Risks / open questions

Blocking
- B1 Migration numbering collision with `t3code/basert-notch-automation-ux` — must reconcile first (§M).
- B2 Gmail restricted-scope distribution posture (own-project/test-user vs CASA
  assessment) — product decision required before Phase 3 UX commitments.

High
- H1 Envelope Index lacks Message-ID in metadata queries → Apple↔Gmail dedupe
  depends on emlx parse availability; offline-unhydrated Apple rows won't
  retro-link until hydrated. Mitigation: hydrate-on-ingest for new messages.
- H2 TCC/FDA attribution differences for launchd-run `bagentd` (known risk
  pattern from notifications work; applies to emlx reading already solved, but
  retest after any packaging change).
- H3 Qwen3-4B classification reliability for alert buckets — mitigate with the
  deterministic prefilter doing most work and strict JSON schemas + repair
  round (evidence-orchestrator precedent).

Medium
- M1 `historyId` expiry shorter than a week in rare cases → bounded resync
  cost; acceptable.
- M2 Banner/notch geometry crowding with three alert sources — capped designs
  (§K) but needs visual QA.
- M3 Keychain custody for Gmail tokens inside `bagentd` — precedent exists but
  needs the launchd-grant empirical check (same as H2).

Low
- L1 Legacy V2 `messages` drop surprises hypothetical external readers.
- L2 Slack native-app deep links deferred.
- L3 Watch/Pub/Sub revisit if sub-minute alert latency ever demanded.

---

## P. Recommended first PR

**Scope:** Phase 1 only — canonical mail foundation, Apple Mail only.

- Migration `V-next__canonical_mail.sql` (exact number per §M outcome):
  creates `mail_accounts`, `mail_messages`, `mail_source_refs`,
  `mail_sync_state`; seeds the `apple_mail` account; **drops the unused V2
  `messages` table**; leaves `mail_cache` untouched.
- New `crates/daemon/src/mail_store.rs`: insert-or-ref logic (Message-ID
  normalize/dedupe), cursor advance via `max(received_at)`, window query
  `[now−24h, now)` excluding Sent/Drafts/Trash/Junk.
- Rewrite `mail_sync_inner` to write canonical rows (keep cache double-write
  so existing chat-context paths don't change behavior in this PR).
- Docs corrections bundled: DATA_MODEL.md mail section replaced with actual
  schema; CONNECTORS.md mail tool list corrected.
- Non-goals (explicit): no Gmail, no briefing automation, no alerts, no Swift
  changes, no new endpoints beyond maybe a debug `GET /mail/canonical/stats`.
- Tests: mail_store unit tests (dedupe, refs, cursor skew, window edges),
  sync idempotence test with fixture Envelope-Index-like DB, regression:
  existing daemon tests stay green.
- Size: ~1 migration, 1 new module, surgical edits to `main.rs` sync path —
  reviewable, reversible, and it locks the architectural seams everything
  else plugs into.

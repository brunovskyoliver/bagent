# Bagent Deep-Research Report — Architecture, Worktree Map, and Product Direction

Date: 2026-08-22 · Evidence-based audit of `main` @342be48, all t3code worktrees,
and `docs/*`. Market sections draw on landscape knowledge (no live web access in
this session); every claim about Bagent itself is cited to code.

---

## 1. Executive summary

Bagent is far more mature than its "notch chatbot" framing suggests. It is a
**local-first macOS personal agent platform**: a launchd-resident Rust daemon
running an agentic tool loop over ~24 native tools, gated by a rules engine and
human-approval framework, with connectors into Mail, Notes, Files, WhatsApp,
Odoo, the web, macOS Notification Center, a bagent-owned WebKit browser, and
external coding agents (Codex/Claude via cmux hooks). The scheduler is
production-grade (DST-correct recurrence, catch-up policies, atomic claims,
unattended safety).

Three findings dominate everything else:

1. **The most advanced architecture is not on `main`.**
   `t3code/basert-notch-automation-ux` carries a 36-commit "Wayfinder" rebuild —
   a daemon-owned **Work Coordinator** as sole authority over foreground chats
   *and* automation runs, a **Model Runtime** with residency leases, persisted
   **Automation Sessions** continuable by chat, and a rethought notch UX (Stage
   Rail, Compass Rail, NotchProjection). Main has 18 commits the branch lacks
   (bagent Browser, notification mirror, reference resolution).
   **Reconcile before any new feature work** — every idea below is cheaper on
   the unified foundation.

2. **Memory was built, then deliberately switched off.**
   Hybrid BM25+vector retrieval (`crates/memory`, 1,274 lines), implicit
   correction capture (`memory_extractor.rs`, `feedback.rs`), and the memory API
   all exist — but `PromptBuilder` is documented *"stateless… intentionally
   ignored"* (`crates/agent/src/prompt.rs:1-8`) and the daemon hardcodes
   `let selected_memory = Vec::new()` (`crates/daemon/src/main.rs:2321`).
   Largest latent capability in the repo.

3. **The automation engine is time-triggered only.** The "notices → reasons →
   proposes → acts" loop needs *event* triggers. Primitives are nearly all
   present: 30-second notification mirror poller (`notifications.rs`), mail sync
   polling, SSE event bus, approval provenance, unattended gating. Missing piece
   is a small trigger-evaluation layer.

Recommendation in one sentence: **merge the Wayfinder line into main, then
evolve bagent into an "ambient automation OS" whose killer feature is persistent
watchers** — natural-language conditions evaluated continuously against mirrored
data (notifications, mail, files) that surface through the notch only when
something meaningful happens.

---

## 2. Current architecture (verified)

### Execution path

```
⌥Space / click → BagentPanel (NSPanel, NotchWrapShape wings+bridge)
  → ChatViewModel (@MainActor; daemon calls; screen pre-gate; attachments)
  → POST /chat (bearer auth) → PromptBuilder::build (layered system prompt,
     skills, attachment/screen ctx, live tool context from classifiers)
  → agentic loop (crates/daemon/src/agent_exec.rs:
     MAX_ROUNDS=5, MAX_TOOL_CALLS=8 — "ponytail: flat budgets" note at :4222)
      → BaseRT Qwen3-4B streaming (127.0.0.1:8082, Metal-local)
      → ToolDef calls → dispatcher gates via RuleEngine (auto/ask/forbidden,
        hot-reloaded YAML), PathPolicy, pending_approvals + oneshot channel
      → audit_entries row per call → result fed back as role:"tool"
  → SSE typed events → notch renders (deltas, mail_found/file_found chips,
    task_rating badge, approvals preempting all surfaces)
```

### Key structural facts

| Component | Location | Notes |
|---|---|---|
| Daemon | `crates/daemon/src/main.rs` (4,805 LOC) + `agent_exec.rs` (8,692) | tool registry + dispatch helpers |
| Scheduler | `crates/daemon/src/scheduler.rs` (843) | 60 s chunked sleeps, atomic claims, 2-run semaphore, restart recovery |
| Automations semantics | `crates/automations` (~850 LOC) | typed schedules, IANA zones, DST |
| Rules | `crates/rules` — YAML, 5 s mtime hot-reload, first-match-wins | read-only defaults `auto` |
| Memory | `crates/memory` (lib 1,274 + selector + markdown_mirror) | FTS5 + embeddings + 0.4·BM25 + 0.6·cosine + recency decay — **disabled in chat path** |
| Classifiers | `crates/agent/src/{mail_intent, window_intent, screen_intent, context_planner, task_rater}` | coreference via last-4-turn snippet |
| Web evidence | `crates/daemon/src/evidence/*` (planner, orchestrator, validator, synthesis, diagnostics) | implements CONTEXT.md evidence language |
| Notification mirror | `crates/daemon/src/notifications.rs` (1,226) | polls `group.com.apple.usernoted/db2/db` every 30 s into SQLite, Slovak case folding, denylist, 30-day purge |
| Browser | `apps/macos/Sources/bagent/Browser*.swift` (~3,500 LOC) + `crates/browser_mcp` | WebKit, isolated profile, loopback/private-net allowlist enforced below MCP, Browser Cues |
| Clipboard | `ClipboardHistory.swift` + `PasteEventTap.swift` + `PasteWheelView.swift` | right-⌘ hold ≥0.3 s → 5-slot repaste wheel, lossless representations |
| External agents | `CmuxNotificationController.swift` | cmux hooks (AskUserQuestion/PermissionRequest/Stop) become notch attention cues |

### Security posture

Model output never authorizes anything: dispatcher enforces rules, `PathPolicy`
denies `.ssh`/Keychains/dangerous extensions, side-effecting tools escalate
`auto→ask` when unattended, unknown tools fail closed, approvals carry origin
JSON and die on restart (`docs/AUTOMATIONS.md`). Notification text renders as
untrusted attributed data. This discipline is the moat.

---

## 3. Feature inventory

| Area | Capability | State |
|---|---|---|
| Notch UI | notch-wrap panel, hover wings, input-first mode, markdown, paste-wheel drag target, approval modal, Browser Cue deck | ✅ mature |
| Conversation | SSE streaming, session persistence, history+summarization, follow-up refs, SK/EN diacritics, slash commands | ✅ mature |
| Agent loop | native tool calling, 5×8 budget, corrective results, per-call audit | ✅; 🟡 flat budgets |
| Tools | mail_*, notes_*, filesystem_*, macos_*, web_search/fetch (+Tavily), notifications_search, whatsapp_*, odoo_*, codex.run_task | ✅ working |
| Automations | CRUD + editor, 6 schedule kinds, catch-up ≤24 h, overlap skip, run history, approval provenance | ✅; 🔴 time triggers only; 🟡 per-run models |
| Background | launchd residency, `/events` SSE, approval preemption | ✅ |
| Memory | store, FTS5+vector hybrid, corrections, glossary, pruning, markdown mirror | 💤 built then disabled (`stateless_no_recall`) |
| Context awareness | ScreenCaptureKit OCR + AX selection + active app, vision routing | ✅ keyword-gated |
| Web evidence | planner/orchestrator/validator/synthesis, citations, corroboration | ✅ unusual depth |
| Browser | hidden sessions, cue interaction, allowlist, submission grants vocabulary | ✅ new on main; roadmap phases open |
| External agents | cmux hook cues; Codex connector w/ deterministic rater | ✅ distinctive |
| Data analysis | ad-hoc only (read_text CSV/xlsx, invoice-analysis skill) | 🟡 thin |
| Visualization | none beyond bubbles/chips/run lists | 🔴 |
| Observability | append-only audit, privacy-safe diagnostic traces | ✅ strong |

**Unusually good:** grounding discipline enforced in code; unattended-safety
design; one-panel geometry contracts; Slovak correctness down to ASCII-folding
in SQL mirrors.

**Underdeveloped:** memory/personalization; event-driven anything; persistent
analysis datasets; visualization; multi-source correlation.

---

## 4. Git worktree map

| Worktree | Branch | vs main | Verdict |
|---|---|---|---|
| `/Users/oliver/Programming/bagent` | `main` @342be48 | — | release line: Browser + notifications + ref-resolution |
| `t3code-183f19bf` | `t3code/basert-notch-automation-ux` | **36 ahead / 18 behind**, tip 2026-08-21 | ⚠️ **most advanced implementation**: `work_coordinator.rs` (2,894), `model_runtime.rs` (1,852), `automation_sessions.rs` (1,342), `current_chat.rs` (1,543), `unified_work.rs`, `ui_relaunch.rs`, `cutover.rs`; Swift: `CompassRail.swift`, `NotchProjection.swift`, `NotchEventConsumer.swift`, `AutomationSplitView.swift`, `PermissionGrantAssist.swift`, `StageRailView.swift`; STAGE1–8 acceptance docs. Lacks main's Browser/notification/ref-resolution commits |
| `t3code-9a5fc6a4` | design-conversational-reference-resolver | 0/0 | planning-only, merged |
| `t3code-ee2c934a` | plan-mail-ingest-alerts | 0/0 | planning-only; name already anticipates the watcher direction |
| `t3code-0a4ba83f` | plan-bagent-notifications | 0/1 behind | merged |
| `t3code-4badc843` | add-embedded-safari-browser | 0/5 behind | merged |
| `t3code-72200c46` | ollama-macbook-models | 0/127 behind | abandoned |

**Conflicting implementations:** automations UX (step-editor vs split-view
"Dvojpanelový prehľad"), chat authority (client-owned vs daemon-authoritative),
daemon layout (monolith vs factored modules). The branch is newer, decision-doc-
driven, acceptance-gated — treat **it** as destination and port main's three
missing features onto it.

---

## 5. Hidden or unfinished capabilities

- Disabled memory stack incl. unused `markdown_mirror.rs` human-readable vault.
- `plan-mail-ingest-alerts` branch — intent without implementation.
- Khoj-over-Tailscale research doc → multi-device ambition on file.
- `ui_relaunch.rs` (648 LOC, branch) — daemon-driven UI-only relaunch fencing.
- Flat budgets flagged in-code (`agent_exec.rs:4222`) — deep research hits them.
- Browser roadmap phases beyond Phase 0 specified but not built.
- AeroSpace control keyword-gated instead of first-class tools.

---

## 6. Architecture unlocks (existing 80% → missing 20%)

**U1 — Event-triggered automations ("watchers").**
Existing: scheduler instant wake, shared agent-exec service, unattended gating,
approval provenance, 30 s notification poll, mail sync poll, `/events`.
Missing: `triggers` table + evaluator, NL→predicate compilation via existing
`generate_json` classifier pattern, cooldown/hysteresis, second schedule kind.
Result: *"when Novem emails about a faktúra, read it, check Odoo, draft a reply
for approval."* Medium difficulty · Very High value.

**U2 — Live agent activity island.**
Existing (branch): Work Coordinator snapshots/events, Stage Rail
(`Model→Think→Tool→Done`), invariant pill, bridge-height attention pricing.
Missing on main entirely; mostly done on branch. Very High value.

**U3 — Scheduled analyst reports with citations.**
Existing: evidence pipeline, run summaries + output reuse, `NotchMarkdown`.
Missing: structured report artifact (sections/tables/citations) + trend storage.
Medium-Low difficulty · High value.

**U4 — Selective memory re-enablement.**
Existing: hybrid retrieval, dedup, pruning, correction classifiers — tested.
Missing: namespace policy decision, injection point, settings kill-switch.
Low difficulty · High value.

**U5 — Clipboard intelligence.**
Existing: lossless pasteboard capture, attachments extraction pipeline, drop
zone. Missing: "analyze clipboard" route + wheel hover action. Low-Medium ·
Medium-High.

**U6 — Browser-powered web monitoring.**
Existing: hidden WebKit sessions, hard-blocked allowlist, cue states.
Missing: allowlist config mechanism + watcher trigger driving browser sessions.
Medium-High · High.

---

## 7–9. Market landscape, trends, what NOT to copy

**Patterns worth stealing**

- Raycast AI / Spotlight: commands as first-class citizens (only 2 slash
  commands today).
- Limitless/Rewind: passive capture + "what did I miss" recall — bagent's
  notification mirror is the consent-forward version.
- Lindy/Zapier Agents/n8n: triggers, templates, replayable run logs — bagent's
  run history + approval provenance is the honest version; templates are cheap.
- Claude Code/Open Interpreter: long-horizon loops, checkpointing, diff
  approvals — Work Coordinator slots/residency leases are the right substrate.
- Apple Intelligence: App Intents trend could feed future triggers.
- Khoj/local-RAG tools: personal knowledge indexing — relevant to U4.
- MCP ecosystem: bagent speaks MCP twice already; becoming a generic MCP host
  is low-novelty-risk ecosystem leverage.

**What Bagent should NOT copy**

- ❌ Cloud-first memory/profiles (contradicts the trust model that makes FDA
  grants acceptable).
- ❌ Voice-first interaction (tried, consciously removed in Phase 5G).
- ❌ Always-on screen recording à la Rewind (ScreenContextProvider is
  deliberately on-demand).
- ❌ Window proliferation (the one-panel rule is correct).
- ❌ Autonomous writes without fresh approvals even for "trusted" watchers.
- ❌ n8n-style visual canvas editor (wrong abstraction for the notch;
  NL + typed steps fits).

---

## 10. Automation opportunities (ranked)

1. Watchers (U1) — start with notifications + mail sources.
2. Chained/dependent automations (Work Coordinator admission queue = fair
   scheduling for free).
3. Automation templates gallery — 5 canned Slovak templates as one-click
   creates (faktúra watch, ranný súhrn, Odoo dlžníci, CI/build watch, nové
   WhatsApp správy od X).
4. Run continuation as chat — designed on branch
   (`AUTOMATION_SESSION_POLICY_DECISION.md`); market it after merge.
5. Escalation & retry semantics — optional backoff + notify-on-failure (itself
   a watcher).
6. Digest grouping — multiple watcher hits collapse into one briefing.

---

## 11. Data-analysis opportunities

- Persistent datasets: register a CSV/JSON path; scheduled re-analysis; diffs
  between runs become insights.
- Personal signals tables: audit log + run history + notification mirror are
  time series nobody visualizes; tiny daily rollups enable trend/anomaly prompts.
- Report artifacts (U3) with tables via `NotchMarkdown`.
- Repo analytics via filesystem connector + git presence.
- Skip: generic BI dashboards; bank/spending integrations v1.

---

## 12. Notch-native opportunities

Filter: *why is the notch better than a window?* Because it is glanceable,
zero-launch, and interruption-priced by geometry (bridge-height table literally
prices attention in points).

1. Activity Peek Stage Rail (branch) — decided and built there.
2. Approval inbox with provenance; add batch-deny.
3. Signal chips on collapsed wings (unread completions, watcher hits, pending
   approvals); idle stays blank per polish rule.
4. Quick capture scratch inbox processed by automations later.
5. Drop-zone triage hover actions (Extract / Analyze / File-under).
6. Clipboard chip "Analyzovať schránku".
7. Mini-report carousel cycling completed analyst runs (98 pt completion height
   already reserved).

---

## 13. Top feature ideas (evidence-backed)

### A — Watchers: event-triggered automations
Editor "Kedy" gains "Keď…"; NL condition compiled once into
`{source: notifications|mail|filesystem, predicate}`; evaluated per poll tick;
executes through unchanged `agent_exec`. Reuses scheduler wake infrastructure,
gating, provenance. Inspired by Lindy/n8n/iOS Shortcuts/Limitless.
Differentiator: fully local, approval-gated, Slovak-native, can watch
Notification Center (no cloud competitor can).
Blocks: triggers table, evaluator, cooldowns, editor step.
Leverage ~85% · Complexity Medium · Value Very High.
Risks: trigger storms (cooldown + digest), predicate drift (audit evaluations).

### B — Merge Wayfinder (Work Coordinator + notch projection)
Prerequisite, not just a feature. Residency leases keep TTFT low for watcher
bursts. Complexity High (36/18 divergence) · Value Very High. Mitigate merge
pain by porting main's 3 features onto the branch.

### C — Selective memory re-enablement
Enable `user_pref` + `sk_glossary` + `style_profile` namespaces only; labeled
prompt layer; Settings toggle + "zabudni všetko". All components previously
acceptance-tested (TODO Phase 4C/4D). Complexity Low · Value High. Keep
cap of 3 items/namespace (already implemented).

### D — Analyst reports & signal dashboard
Structured report artifact + last-N-runs trend line in notch detail view;
weekly digest template. Leverage evidence synthesis + markdown renderer +
retention. Complexity Medium · Value High.

### E — Clipboard intelligence
Paste-wheel hover action → analyze current item via attachments pipeline.
Complexity Low-Medium · Value Medium-High.

### F — Generic MCP host
Settings page registers arbitrary MCP stdio servers; tools appear with rules
default `ask`; per-server enable required to protect fail-closed discipline.
Complexity Medium · Value Medium.

### G — Morning briefing composite automation
Aggregates overnight notifications, unread mail, watcher hits → single notch
card at wake time. Pure composition of A+B+D. Complexity Low post-A/B · Value
High visibility.

---

## 14. Prioritized matrix

| Idea | User value | Differentiation | Existing leverage | Complexity | Priority |
|---|---:|---:|---:|---:|---|
| B — Wayfinder merge | ★★★★★ | ★★★★ | ★★★★★ | High | **P0** |
| A — Watchers | ★★★★★ | ★★★★★ | ★★★★ | Medium | **P1** |
| C — Selective memory | ★★★★ | ★★★ | ★★★★★ | Low | **P1** |
| G — Morning briefing | ★★★★ | ★★★★ | ★★★★ | Low (post-A) | P2 |
| D — Reports/signals | ★★★★ | ★★★★ | ★★★★ | Medium | P2 |
| E — Clipboard IQ | ★★★ | ★★★ | ★★★★ | Low-Med | P2 |
| F — MCP host | ★★★ | ★★★★ | ★★★ | Medium | P3 |

Quick wins (<1 week): adaptive budgets for deep-research turns; per-automation
model selection (documented limitation); AeroSpace promotion to tools;
batch-deny approvals; watcher dry-run logging.

Probably skip: voice revival, always-on screen recording, cloud sync, canvas
editor, weather/trivia skills, fine-tunes.

---

## 15–17. Direction and phased roadmap

### Recommended direction: **Ambient Automation OS**
Local, approval-gated agent runtime living in the notch: watches your digital
life, reasons when something happens, acts with permission, reports glancibly.
Secondary flavor: **Personal Intelligence Layer** (C+D) rides along nearly free.
Derived from assets, not asserted: strongest differentiators are exactly (a)
daemon residency, (b) safety-gated execution, (c) notch projection,
(d) automations — and mainstream coverage of proactive-but-accountable local
automation is weakest precisely there.

**Phase 0 — Reunify.** Port Browser, notification mirror, reference resolution
onto `basert-notch-automation-ux`; adopt branch module layout; remove legacy
paths flagged by `cutover.rs`. Highest-risk item; do it while divergence is 36
commits, not 100.

**Phase 1 — Personalization + headroom.** Selective memory (C); adaptive
budgets; per-automation models. Files: `prompt.rs`, `agent_exec.rs`,
`automations_api.rs`, settings.

**Phase 2 — Watchers.** Triggers schema + evaluator + cooldowns + editor step +
audit; notifications + mail sources first, filesystem mtime second; then G and
E. Files: `scheduler.rs`, new `triggers.rs`, automation UI.

**Phase 3 — Ambient personal intelligence.** Report artifacts + signals rollups
(D); browser-backed watchers (U6) behind allowlist config; templates; MCP host
(F).

**Phase 4 — Platform.** Multi-device (Tailscale/BaseRT listener research on
file), longer-horizon loops on Work Coordinator slots, submission-grant browser
flows.

---

## 18. Top 5 things to build next

1. Merge/port Wayfinder into main (Phase 0).
2. Watchers: event-triggered automations from notification mirror + mail sync.
3. Selective memory re-enable with namespaces + kill-switch.
4. Morning briefing composite automation on top of watchers.
5. Analyst report artifacts with notch-rendered tables/trends.

---

## 19. Open technical questions / risks

- Can `bagentd` reliably hold Full Disk Access across updates (TCC
  responsible-parent asymmetry in `NOTIFICATIONS_PLAN.md`)? Watchers inherit it.
- Trigger-predicate quality from Qwen3-4B: needs dry-run mode + audited
  evaluations + easy edit-back-to-DSL.
- Continuous-evaluation cost: cheap local heuristics for candidates, model only
  for confirmation.
- Phase 0 merge risk: consider mechanical port order (browser → notifications →
  ref-resolution), STAGE-style acceptance docs per port.
- Autonomy creep: never "auto-approve trusted watchers"; fresh approvals are
  the brand.
- SQLite strain: `audit_entries` never pruned; retention strategy needed before
  Phase 3.

---

## 20. Concrete next actions

1. Read branch issue notes for `plan-mail-ingest-alerts` before designing
   triggers.
2. Decide port order for Phase 0 and cut a tagged reunification release.
3. Prototype a hardcoded watcher (notification match X → task Y, cooldown) to
   de-risk the schema.
4. Flip selective memory behind a toggle; measure answer quality delta.
5. Sketch the report-artifact JSON shape against the evidence pipeline's
   synthesis output.

---

## If I had one month

Week 1: reunification (port Browser, notifications, reference resolution onto
the Wayfinder branch; tagged release with STAGE-style acceptance docs).
Week 2: selective memory + adaptive budgets + per-automation models.
Weeks 3–4: watchers end-to-end — triggers table, evaluator over notification
mirror + mail sync feeds, cooldowns, editor step, audit trail, plus Morning
Briefing as flagship template. Ship with one demo: *overnight Slack ping about
a faktúra → morning notch briefing → tap → full cited analysis → drafted reply
awaiting approval.*

## If I had one week

Hardcoded prototype watcher ("new notification matching X → run task Y",
cooldown, riding existing scheduler wake and `agent_exec`). Proves the
interaction and de-risks the Phase 2 schema. Fallback if the branch feels
unstable: flip selective memory behind a toggle — smallest diff, biggest
per-conversation payoff.

## The killer feature

**The Watcher: natural-language, locally-evaluated, approval-gated event
automations surfacing only through the notch.** Every competitor owns a
fragment: Zapier has triggers but is clouded and tool-shallow; Raycast is
interactive but not resident; Limitless captures but doesn't act; Siri is
proactive but opaque. Bagent uniquely owns all four layers: resident daemon with
privileged local feeds (Notification Center, Mail SQLite, filesystem), safe
execution with human gates, a model reasoning in Slovak over what happened, and
a glanceable surface whose entire grammar is already an interruption-cost budget
(blank idle, 98 pt completion, 176 pt approval). Nobody can copy this without
first rebuilding the trust model — which took nine stages.

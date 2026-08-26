# Reliable grounded Mail and web flows: executable redesign specification

## 1. Purpose and scope

This specification replaces prompt-dependent Mail and web evidence acquisition with a
typed, deterministic path shared by interactive chat and automations. Models interpret
and summarize evidence; they do not own required evidence transitions.

The change is limited to read-only Apple Mail and web requests. It preserves:

- the shared foreground/automation execution service;
- the existing rules-engine gate and approval semantics;
- stateless chat construction;
- the general native tool loop for unrelated connectors; and
- the rule that connector content is untrusted data.

The implementation starts at `crates/daemon/src/agent_exec.rs`. Recognized Mail or web
turns branch into the Evidence Orchestrator before the existing model-controlled loop.
Unrecognized and unrelated turns continue through `run_agent_loop` unchanged.

## 2. Why this change is required

The production prompt `can you read me the 3 latest emails?` was broadly routed to Mail,
but `desired_mail_read_count` recognized only summary wording. The deterministic prefetch
did not run. Qwen3 4B issued three inbox listings, read no bodies, and returned an
unfinished promise. The UI converted those calls into `3 steps completed`.

Web acquisition is likewise guidance in prompts rather than an enforced transition.
Current search and fetch functions return prose, so provider failure, empty results,
challenge pages, redirects, unsupported content, and valid evidence are not reliably
distinguishable.

The installed-model matrix confirms this cannot be repaired by choosing a larger model:

| Model | Mail result | Web result | Admission |
| --- | --- | --- | --- |
| Qwen3 4B Instruct | Synthetic 3/3, but known production failure | Synthetic 3/3 | Typed-intent proposal and synthesis fallback only |
| Qwen3 8B | 0/3; 100.8–141.7s; invented rowids | One production pass | Not admitted interactively |
| Qwen3.6 35B-A3B | 0/3 required sequence; transcript incompatibility | 0/3 required sequence | Preferred tool-free synthesis |

## 3. Component boundary

Add an `evidence` module under `crates/daemon/src/` with these responsibilities:

```text
EvidenceIntentClassifier
    user request -> ClassifiedIntent

EvidencePlanner
    ClassifiedIntent -> EvidencePlan

EvidenceOrchestrator
    plan -> policy-gated operations -> typed connector results

EvidenceValidator
    results + plan -> EvidenceBundle | RecoveryOutcome

SynthesisService
    eligible bundle -> 35B/4B answer -> validated answer or deterministic rendering

DiagnosticRecorder
    phase, logical activity, attempt, evidence delta, validation, outcome
```

`EvidenceOrchestrator` is the facade used by both chat and automations. It depends on
adapters for Mail, web, policy admission, inference, events, clock, and diagnostics.
It must not depend on Swift UI types or parse model-generated raw connector arguments.

Recommended Rust entry point:

```rust
async fn execute_evidence_turn(
    ctx: EvidenceContext<'_>,
    request: EvidenceRequest,
) -> Result<EvidenceTurnOutcome, EvidenceExecError>;
```

`EvidenceContext` carries the current `AppState`, `EventSink`, `ExecOrigin`,
session/turn identifiers, and model runtime. Each operation invokes the existing `Gate`
immediately before connector execution. Planning does not grant authority.

`run_agent_loop` performs deterministic classification before calling inference:

```text
recognized Mail/web -> execute_evidence_turn
ambiguous sensitive scope -> clarification outcome
unrecognized request -> existing general agent loop
```

Do not retain the current Mail-only prefetch block, follow-up system messages, or
`mail_tool_succeeded` string heuristics after the new path reaches parity.

## 4. Canonical data shapes

Use versioned, serde-serializable internal types. Exact Rust field spelling may follow
repository conventions, but all semantics below are required.

```rust
struct EvidenceRequest {
    version: u16,
    turn_id: String,
    session_id: String,
    original_text: String,       // in memory only; never diagnostic persistence
    origin: EvidenceOrigin,      // Chat | Automation
}

enum EvidenceIntent {
    MailLatestHeaders { count: u8, unread_only: bool },
    MailLatestContent { count: u8, unread_only: bool },
    MailTargeted { query: String, needs_content: bool },
    WebDirectPage { url: Url },
    WebFact {
        query: String,
        verification: VerificationLevel,
    },
}

enum VerificationLevel {
    SingleAuthoritative,
    Corroborated,
}

enum Classification {
    Recognized(EvidenceIntent),
    NeedsClarification { prompt: String, alternatives: Vec<IntentSummary> },
    NotEvidenceIntent,
}

struct EvidencePlan {
    version: u16,
    intent: EvidenceIntent,
    requirements: Vec<EvidenceRequirement>,
    budget: EvidenceBudget,
}

struct EvidenceBudget {
    mail_list_attempts: u8,      // 1
    mail_body_attempts: u8,      // <= 10
    web_search_attempts: u8,     // <= 2
    web_fetch_attempts: u8,      // <= 5
    max_parallel_fetches: u8,    // 2
    optional_exploration_rounds: u8, // 1
}
```

Every executable operation has a canonical identity:

```rust
struct OperationKey(String);

enum EvidenceOperation {
    MailList { limit: u8, unread_only: bool },
    MailSearch { normalized_query: String, limit: u8 },
    MailRead { message_id: ValidatedMailId },
    WebSearch { normalized_query: String, provider_set: ProviderSet },
    WebFetch { candidate_id: CandidateId },
}
```

Generate `OperationKey` from normalized operation type and validated arguments.
It groups attempts and suppresses duplicate execution within a turn. Never expose raw
Mail rowids or arbitrary URLs to a model.

### 4.1 Typed execution results

```rust
enum ExecutionStatus {
    Succeeded,
    Failed(FailureCode),
    Denied,
    TimedOut,
}

enum FailureCode {
    InvalidInput,
    ConnectorUnavailable,
    ConnectionReset,
    RateLimited,
    Http4xx(u16),
    Http5xx(u16),
    UnsupportedContentType,
    UnsafeDestination,
    RedirectUnsafe,
    BodyTooLarge,
    EmptyExtraction,
    ProviderChallenge,
    ParseFailure,
    ModelUnavailable,
    ModelInvalidOutput,
    OtherNormalized,
}

enum EvidenceContribution {
    Satisfied,
    Partial,
    Empty,
    Duplicate,
    Irrelevant,
}

struct OperationResult<T> {
    key: OperationKey,
    attempts: u8,
    execution: ExecutionStatus,
    contribution: EvidenceContribution,
    value: Option<T>,
    duration_ms: u64,
}
```

Retry exactly once only for timeout, connection reset, rate limit, or HTTP 5xx.
An actual retry consumes the same global operation budget as its first attempt.
Permanent, denied, invalid, unsupported, empty, extraction, validation, and safety
failures are not retried. Suppressed duplicates consume no budget.

### 4.2 Mail results

```rust
struct MailHeaderEvidence {
    evidence_id: EvidenceId,
    connector_id: ValidatedMailId,
    sender: String,
    subject: String,
    received_at: DateTime<Utc>,
}

struct MailBodyEvidence {
    evidence_id: EvidenceId,
    header_id: EvidenceId,
    body: String,                // bundle memory only
    body_state: BodyState,
}

enum BodyState {
    Readable,
    UnavailableLocally,
    Empty,
}
```

Connector identifiers remain trusted adapter values. Synthesis sees `EvidenceId`, never
the raw rowid. Empty query, unavailable connector, and policy denial are separate states.

### 4.3 Web results

```rust
struct WebSearchResult {
    providers: Vec<ProviderResult>,
    candidates: Vec<WebCandidate>,
}

struct ProviderResult {
    provider: WebProvider,
    status: ProviderStatus,
    duration_ms: u64,
}

enum ProviderStatus {
    Succeeded { result_count: u16 },
    Empty,
    Challenged,
    Failed(FailureCode),
}

struct WebCandidate {
    candidate_id: CandidateId,
    provider: WebProvider,
    rank: u16,
    title: String,
    requested_url: Url,
    snippet: String,             // discovery only, never factual evidence
}

struct WebFetchEvidence {
    evidence_id: EvidenceId,
    candidate_id: CandidateId,
    requested_url: Url,
    final_url: Url,
    redirect_chain: Vec<Url>,
    http_status: u16,
    content_type: String,
    bytes_read: u64,
    characters_extracted: u64,
    extraction: ExtractionStatus,
    passages: Vec<EvidencePassage>,
    links: Vec<ValidatedReference>,
}

enum ExtractionStatus {
    Readable,
    ReadableTruncated,
    Empty,
    Unsupported,
}
```

Only `Readable` or `ReadableTruncated` fetches with useful passages become Fetched
Evidence or citation targets. Citations use `final_url`. Validate every redirect hop and
the final resolved IP against the private/local destination policy. Re-resolve or pin the
validated address for connection to prevent hostname-to-private-address and DNS-rebinding
bypasses.

## 5. Deterministic intent rules

Classification is conservative and multilingual enough to preserve current English and
Slovak use.

- Mail nouns plus latest/recent/last wording select a latest-Mail intent.
- Content verbs such as `read`, `what is in`, `summarize`, `prečítaj`, `zhrň`, and their
  normalized variants select `MailLatestContent`; absence of content language selects
  `MailLatestHeaders`.
- Parse an explicit positive count and clamp a Reading Batch to 10. Default plural latest
  Mail requests to 3 and singular requests to 1.
- Sender, subject, or conversation wording selects `MailTargeted`. Execute an exact
  Targeted Mail Match only when one normalized candidate is unambiguous.
- An explicit HTTP(S) URL selects `WebDirectPage`.
- A request for current or externally verifiable facts selects `WebFact`.
- Mark fast-changing, comparative, conflicting, or consequential requests
  `Corroborated`; simple direct facts use `SingleAuthoritative`.

If plausible intents materially differ in privacy, operation cost, or evidence scope,
return one clarification prompt with safe alternatives. Do not silently choose the
broader plan.

For wording outside deterministic coverage, Qwen3 4B may propose one JSON
`EvidenceIntent`. Validate all extracted values and prove that the proposed intent is
entailed by the request. Invalid or broader proposals become clarification or
`NotEvidenceIntent`; they never directly execute.

## 6. State machine

```text
Received
  -> Classifying
  -> ClarificationRequired | Planning
Planning
  -> Acquiring
Acquiring
  -> Denied | Empty | Unavailable | Partial | Verifying
Verifying
  -> Recovery | Exploring | BundleReady
Exploring (at most once)
  -> Acquiring within remaining budget -> Verifying
BundleReady
  -> LoadingModel (cold only) -> Synthesizing
Synthesizing
  -> ValidatingAnswer
ValidatingAnswer
  -> Complete
  -> Repairing (at most once for invalid output)
  -> FallbackSynthesizing (availability/timeout only)
Repairing | FallbackSynthesizing
  -> ValidatingAnswer | DeterministicRendering
Recovery | DeterministicRendering
  -> Complete
```

All terminal paths emit exactly one Evidence Outcome. State transitions are owned by the
orchestrator, not inferred from prose or tool-call counts.

## 7. Mail flows

### 7.1 Latest headers

1. Classify count and unread filter.
2. Gate `MailList` immediately before execution.
3. Return sender, subject, and date from up to the requested count.
4. Do not read bodies or invoke synthesis unless the user requested interpretation.
5. Distinguish a successful empty inbox query, unavailable connector, and denied access.

### 7.2 Latest content

1. Clamp the requested Reading Batch to ten and record excess as a shortfall with an offer
   to continue.
2. Gate and execute one newest-first inbox listing.
3. For each distinct validated returned identifier, gate and execute one sequential
   `MailRead`.
4. Record readable bodies, unavailable bodies, and denials independently. Never replace a
   missing body with its snippet or model knowledge.
5. A bundle with at least one readable body is Partial or Complete and synthesis-eligible.
   A bundle with no readable body becomes a deterministic Recovery Outcome.
6. The exact prompt `can you read me the 3 latest emails?` must execute one list followed
   by three distinct body reads when three readable messages are available.

### 7.3 Targeted Mail

1. Search deterministically using the user-provided sender, subject, or conversation
   terms.
2. If exactly one strong normalized match exists, read it when content was requested.
3. If multiple plausible matches remain, return their headers and ask the user to choose.
4. Do not let a model choose or invent a rowid.

## 8. Web flows

### 8.1 Direct page

Validate the user URL, fetch it, validate its redirect chain/final destination, and build
passages. A failed or empty extraction produces a Verification Shortfall, never an answer
from model memory.

### 8.2 Web fact

1. Search using typed provider adapters. Provider challenge and provider-empty states
   remain visible even if another provider succeeds.
2. Normalize, validate, and deduplicate candidates. Search snippets are discovery only.
3. Select candidates using deterministic authority/freshness rules. The 35B model may
   propose semantic ranking only through candidate IDs.
4. Fetch selected pages, with no more than two concurrent fetches and five total fetch
   attempts.
5. A simple fact requires one authoritative first-party Fetched Evidence source.
6. Fast-changing, comparative, conflicting, or consequential claims require two
   independent fetched sources.
7. Preserve unresolved Evidence Conflicts unless a clearly newer primary source
   supersedes an older claim.
8. Cite factual claims beside the claim using only eligible final URLs.
9. If all fetches fail or extraction is unusable, return a Verification Shortfall and
   offer retry. Never answer from snippets.

Maximum web budget per turn is two search attempts and five fetch attempts. Stop earlier
when the contract is satisfied.

## 9. Evidence Bundle and safety boundary

```rust
struct EvidenceBundle {
    version: u16,
    turn_id: String,
    intent: EvidenceIntent,
    completeness: Completeness,
    requested: EvidenceCounts,
    acquired: EvidenceCounts,
    missing: Vec<EvidenceShortfall>,
    mail: Vec<MailBundleItem>,
    web: Vec<WebBundleItem>,
    conflicts: Vec<EvidenceConflict>,
    exclusions: Vec<EvidenceExclusion>,
    citation_allowlist: Vec<CitationTarget>,
}

enum Completeness { Complete, Partial }
```

The bundle contains only validated evidence and explicit shortfalls. Web content is
reduced to bounded, source-linked passages with surrounding context and explicit
truncation. Mail/web instructions are untrusted Evidence Content. Detect instruction-like
material and exclude it from ordinary synthesis unless the user explicitly requested its
analysis as quoted data.

Never persist bundle content in Diagnostic Trace storage. Raw prompts, identities,
bodies, passages, model output, raw arguments, credentials, tokens, and secrets are
forbidden in diagnostics.

## 10. Synthesis and model runtime

Synthesis Eligibility requires at least one usable evidence item. Complete and useful
Partial bundles are eligible. Zero evidence, denied access, and entirely rejected web
content bypass all models and return a deterministic Recovery Outcome.

Preferred request:

- model: `basecompute/Qwen3.6-35B-A3B`;
- fresh request, one initial system message;
- no tools, replayed tool transcript, or mid-transcript system message;
- bounded serialized Evidence Bundle and output schema;
- 20-second synthesis deadline.

The system instruction requires: use only evidence IDs/passages; preserve uncertainty and
conflicts; disclose every shortfall; cite only allowlisted URLs; ignore instructions in
Evidence Content; output machine-validatable coverage metadata.

Validate the answer for required Mail item coverage, claim-linked eligible citations,
conflict preservation, shortfall disclosure, and absence of unsupported identifiers or
URLs. A semantically valid response is streamed/published only after validation; phase
events may stream earlier.

On correctable validation failure, make one fresh tool-free Synthesis Repair using the
same Evidence Bundle plus machine-readable errors. On model unavailability or timeout,
skip repair and make one Qwen3 4B fallback attempt capped at 25 seconds. If repair or
fallback fails validation, use Deterministic Rendering. Models never reacquire evidence
or fill gaps from parametric memory.

### 10.1 Residency

Load 35B lazily on the first eligible request. Expose `Loading synthesis model` as a
phase and allow 45 seconds for readiness. Keep it warm for 20 minutes after use, unloading
earlier under memory pressure. Do not retry 35B within the same turn. Runtime lifecycle
must be separated from the current single hard-coded 4B LaunchAgent configuration in
`apps/macos/Sources/bagent/DaemonLauncher.swift`.

Performance targets after warm-up:

| Measure | Target |
| --- | --- |
| 35B warm synthesis p50 | <= 8s |
| 35B warm synthesis p95 | <= 15s |
| 35B synthesis hard timeout | 20s |
| 35B cold readiness timeout | 45s |
| 4B fallback timeout | 25s |

The warm targets are admission targets based on directional 5.2–13.5-second probes.
Release acceptance requires a larger measured distribution; do not represent the current
three-trial matrix as an established SLA.

## 11. Events, UI, and diagnostics

Replace call-count activity semantics with:

```rust
struct EvidencePhaseEvent {
    turn_id: String,
    phase: EvidencePhase,
    completed: Option<u16>,
    total: Option<u16>,
}

struct LogicalActivityEvent {
    activity_id: String,
    operation: NormalizedOperation,
    status: ExecutionStatus,
    contribution: EvidenceContribution,
    evidence_count: u16,
    source_domains: Vec<String>,
    duration_ms: u64,
    attempts: u8,
    duplicates_suppressed: u8,
    failure: Option<FailureCode>,
}

struct EvidenceOutcomeEvent {
    state: OutcomeState, // Verified | Partial | Empty | Unavailable | Denied
    acquired: u16,
    requested: u16,
    source_count: u16,
    message: String,
}
```

Swift decodes these as new `ChatEvent` variants in `DaemonClient.swift`.
`ChatViewModel.swift` updates the current phase and groups attempts by logical activity.
`ChatView.swift` replaces `N steps completed` with outcomes such as:

- `Read 3 of 3 emails`
- `Read 2 of 3 emails · partial`
- `Web verified · 2 sources`
- `Mail access denied`
- `Couldn't verify sources`

Expanded activity shows normalized operation, evidence counts, domains, duration, retry
count, duplicate suppression, contribution, and failure code. Execution success and
Evidence Contribution remain separate. Empty, duplicate, and irrelevant calls do not
advance progress.

Persist a privacy-safe Diagnostic Trace keyed by turn. Record phase transitions,
Operation Key hashes, model IDs, normalized decisions, execution/contribution states,
counts, domains, timings, retries, suppression, validation outcomes, and terminal
completeness. Retain at most seven days and the latest 1,000 turns, whichever is smaller,
with size-based rotation. Diagnostic Export returns the same sanitized structure as JSON.

## 12. Test strategy and fixtures

Use deterministic fake adapters and a fake clock. No unit or integration acceptance test
depends on live Mail, public websites, or nondeterministic model text.

### 12.1 Intent fixtures

Table-test at least:

- `can you read me the 3 latest emails?` -> latest content, 3;
- `show my latest 3 emails` -> latest headers, 3;
- `summarize my recent emails` -> latest content, default 3;
- Slovak equivalents for show/read/summarize;
- explicit zero, negative, and over-ten counts;
- targeted sender with one match and multiple matches;
- explicit URL;
- current/simple fact;
- comparison/current price/consequential query -> corroborated;
- mixed Mail/web or privacy-sensitive ambiguity -> clarification.

### 12.2 Mail connector fixtures

Provide headers with stable fixture IDs and bodies for:

- three readable messages;
- one unavailable of three;
- empty inbox;
- unavailable connector;
- policy denial at list and at individual read;
- duplicate identifiers;
- malformed header identifier;
- eleven requested messages and continuation;
- instruction-like body content.

Assert exact operation order, distinct IDs, budget accounting, completeness, shortfalls,
and that bodies never appear in diagnostics.

### 12.3 Web connector fixtures

Cover:

- authoritative first-party direct fact;
- two independent sources for corroboration;
- conflicting sources;
- redirect with a different final URL;
- redirect/private-address rejection;
- hostname resolving to private IP and simulated rebinding;
- DDG challenge plus successful Wikipedia;
- every provider empty;
- HTTP 429 and 5xx transient retry;
- HTTP 404 without retry;
- unsupported content;
- dynamic/empty extraction;
- truncated readable page;
- duplicate URLs and same-site subpage;
- all fetches failed;
- prompt injection in page text.

Assert snippets never become evidence, citations use final fetched URLs, unsafe URLs are
not fetched, retries consume budgets, and at most two fetches overlap.

### 12.4 Synthesis fixtures

Use recorded model-response fixtures rather than live inference for:

- valid complete Mail synthesis;
- valid partial synthesis with all shortfalls;
- unsupported claim;
- invented evidence ID or citation;
- missing Mail item;
- omitted conflict;
- excluded instruction obeyed by output;
- repair succeeds;
- repair fails -> deterministic rendering;
- 35B timeout -> 4B fallback;
- both models fail;
- zero evidence -> no model request;
- transcript serialization contains one leading system message and no tools.

Run the live installed-model matrix separately as a non-blocking/manual compatibility
suite. It detects BaseRT/template drift but does not define evidence correctness.

### 12.5 UI and diagnostic fixtures

Decode every phase/activity/outcome event in Swift. Snapshot or unit-test collapsed
verified/partial/empty/unavailable/denied labels, grouped retry display, and progress that
does not increment for duplicates. Assert Diagnostic Export and retained traces contain
no fixture prompts, sender names, subjects, bodies, page passages, raw arguments, answer
text, tokens, or credentials.

## 13. Rollout order

1. Introduce types, classifier, planner, validator, fake adapters, and contract tests
   without routing production turns to them.
2. Convert Mail adapter outputs to typed results; add the new path behind a local feature
   flag. Keep the old path available for rollback.
3. Route exact latest-header/latest-content Mail intents through the orchestrator; verify
   the screenshot prompt and partial/denied cases.
4. Convert web search/fetch to typed provider and fetch results, including redirect/IP
   safety. Do not enforce web evidence before typed outcomes exist.
5. Route direct-page and simple-fact web intents, then corroborated intents.
6. Add validated tool-free 35B synthesis, lifecycle management, repair, and 4B fallback.
7. Add evidence phase/outcome UI, Diagnostic Trace retention/export, and remove
   call-count completion wording.
8. Run fixtures, focused Rust/Swift suites, live Mail/web smoke tests, model compatibility
   probes, latency distribution, and memory-pressure tests.
9. Make the new path default after acceptance. Remove the old Mail prefetch/guidance and
   prose-result heuristics only after rollback confidence is established.

The feature flag controls routing, not connector policy. A rollback returns recognized
turns to the prior loop; it must not alter stored user data or rules.

## 14. Acceptance criteria

The redesign is accepted only when all conditions hold:

1. The exact screenshot prompt produces one inbox listing and three distinct reads, then
   a grounded answer, when fixtures/live Mail provide three readable messages.
2. Header-only requests never read bodies.
3. Partial, empty, unavailable, and denied Mail outcomes are distinct and correctly
   disclosed.
4. No factual web answer is emitted from snippets alone; every citation targets a
   successful fetched final URL.
5. Corroborated intents require two independent sources or explicitly remain partial.
6. Mandatory operations occur with a model that emits no tool calls.
7. No model-generated rowid, arbitrary URL, unsupported claim, or unallowlisted citation
   reaches connector execution or the user.
8. Qwen3.6 synthesis requests contain one initial system message, no tools, and no
   mid-transcript system message.
9. 35B timeout/unavailability causes exactly one bounded 4B fallback; zero evidence calls
   neither model.
10. Operation, retry, concurrency, round, cold-load, and synthesis limits are enforced
    and tested.
11. Collapsed UI reports Evidence Outcome, not tool-call count; retries and duplicates
    cannot inflate progress.
12. Sanitized diagnostics explain execution and completeness while containing none of
    the prohibited private content.
13. The same evidence contracts pass through chat and automation entry points.
14. Existing unrelated connector behavior, rules-engine decisions, and approval tests
    remain unchanged.
15. Warm 35B measurements meet the 8s p50 and 15s p95 admission targets on the target
    machine, or rollout remains flagged while measured values and the user-visible impact
    are reviewed.

## 15. Decision-completeness audit

Every destination category is explicit:

| Category | Spec section |
| --- | --- |
| Routing and component responsibility | 3, 5 |
| Interfaces and data shapes | 3, 4, 9 |
| Mail and web flow | 7, 8 |
| State transitions | 6 |
| Safety and policy | 3, 4.3, 9 |
| Failure and partial results | 4, 6–10 |
| Model/transcript compatibility | 2, 10 |
| Observability and privacy | 11 |
| Fixtures and verification | 12 |
| Performance and operation budgets | 4, 8, 10 |
| Rollout and rollback | 13 |
| Acceptance | 14 |

No further product or architecture decision is required before implementation. Exact
Rust module subdivision, enum naming, database migration mechanics, and UI styling are
implementation choices constrained by the contracts above.

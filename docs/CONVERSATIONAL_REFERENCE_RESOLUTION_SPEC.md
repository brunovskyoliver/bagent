# Conversational Reference Resolution Specification

Date: 2026-07-30

Status: Design complete; implementation not authorized

Repository baseline inspected: `53b0c1b906d56a8d742b1c749b1b860e031f5732`

## Executive verdict

Replace the SD-card-specific post-Stage 9 clarification only after a universal
typed `ConversationalReferenceResolver` passes the acceptance gates in this
specification.

The selected architecture is a hybrid of:

- persisted typed mentions with immutable provenance;
- opaque anchors for private and untrusted evidence;
- deterministic candidate eligibility and ambiguity rules;
- optional local-model ordering that cannot change correctness or authority;
- exact, structured confirmation for non-user or provenance-lost terms; and
- a sealed, one-turn provider-query authorization checked immediately before
  every external provider operation.

The resolver is a deep module at the shared chat/automation routing seam. Its
interface hides mention extraction, provenance lookup, sensitivity policy,
confirmation state, query composition, persistence, clocks, HMAC/encryption,
and optional local ranking.

This design is safer and more general than the current workaround because it
does not infer disclosure authority from a phrase such as "that SD card" or
from plain conversation text. It supports products, people, organizations,
places, standards, document titles, URLs, aliases, pronouns, demonstratives,
and generic noun phrases while preserving the rule that private or untrusted
text cannot silently become an external query.

Implementation is **not authorized** by this document. Review and explicit
authorization are required before source, schema, runtime, or UI changes.

## Scope

This specification defines:

- trusted mention provenance across turns;
- deterministic reference resolution;
- external-provider query authorization;
- exact confirmation semantics;
- chat and automation parity;
- persistence, retention, migration, and rollback;
- privacy-safe diagnostics;
- synthetic and signed acceptance gates; and
- the conditions for replacing the SD-card-specific workaround.

This specification does not authorize:

- implementation;
- changing current production routing;
- removing the SD-card workaround;
- creating a database migration;
- adding a Keychain item;
- launching or restarting the app or daemon;
- calling Mail, Tavily, another provider, or a local model; or
- committing or pushing any change.

## Evidence classification

This document separates three kinds of statements:

1. **Repository finding**: directly demonstrated by the inspected source,
   migrations, documentation, Git history, or privacy-safe local structural
   characterization.
2. **External-source finding**: none. Local repository evidence was sufficient,
   so no external technical sources were used.
3. **Architectural inference**: a proposed design consequence derived from the
   repository findings and the required safety rules.

## Proven repository state

At the final pre-document refresh:

- checkout: `/Users/oliver/Programming/bagent`;
- branch: `main`;
- HEAD: `53b0c1b906d56a8d742b1c749b1b860e031f5732`;
- `53b0c1b` and its parent Tavily changes were preserved;
- the highest current migration was `V14__approval_origin.sql`;
- the next apparent migration ordinal was V15, but implementation must
  re-check this after refreshing HEAD; and
- the specification file did not yet exist.

Protected untracked files present and left untouched:

- `.scratch/`
- `CONTEXT.md`
- `bagent_icon.png`
- `docs/KHOJ_BASERT_TAILSCALE_RESEARCH.md`

Required commits inspected:

| Commit | Proven effect |
|---|---|
| `726ddd5` | Added the SD-card-specific unresolved-reference check and deterministic clarification before the legacy model/tool path. |
| `178100c` | Represented that check as `Classification::RequiresPublicProductIdentity` and consumed it at shared routing. |
| `354abf3` | Kept the clarification presentation constant test-only at the `agent_exec` import seam. |

No concurrent Tavily work is part of this design and none may be reverted,
rewritten, or included in a future resolver commit accidentally.

## Proven current architecture

### Chat ingress and conversation history

Repository findings:

- `crates/daemon/src/main.rs` defines `ChatRequest` with a current `message`,
  `session_id`, attachment IDs, ephemeral screen context, and
  `history: Vec<Message>`.
- The accepted history representation contains only `role` and `content`.
- The daemon accepts only `user` and `assistant` roles, clamps the history to
  10 turns, 1,500 characters per turn, and 8,000 characters in total, and
  inserts it between system layers and the current user turn.
- `apps/macos/Sources/bagent/ChatViewModel.swift` constructs that history from
  visible in-memory `ChatMessage` values.
- `apps/macos/Sources/bagent/DaemonClient.swift` serializes each history item
  only as `HistoryTurn { role, content }`.
- Evidence outcome, sources, attachment refs, Mail refs, canonical status, and
  assistant-production lineage are not included in the history wire type.

Consequence:

Plain client-supplied history proves neither durable authorship nor evidence
lineage. It can remain model context, but it is not a trusted resolver input.

### Persistence

Repository findings:

- `V4__sessions_messages_memory.sql` defines `sessions` and `chat_turns`.
- `chat_turns` stores role, content, language, model, timestamp, and parent
  turn, but no evidence or assistant-production provenance.
- The active chat path explicitly states: "Stateless chat: do not persist user
  turns or attachment links."
- `/sessions/:id/turns` returns `GONE` because session history is disabled.
- daemon startup purges `chat_turns`, `chat_turn_attachments`, legacy memory
  items, chat/memory embeddings, session summaries, and session metadata.
- Sessions themselves persist, but their ID alone does not recover
  conversational provenance.

Consequence:

Historical plain-text rows must not be backfilled as safe mentions. Existing
sessions start with zero trusted historical mentions after the resolver is
introduced.

### Runtime connector references

Repository findings:

- `RuntimeRefs` stores the most recent Mail, file, Odoo, and WhatsApp refs.
- `AppState` holds these refs in an in-memory map keyed by session ID.
- Mail refs include private connector material such as rowid, subject, and
  sender.
- These refs are converted into prompt notes for legacy follow-ups.
- The map is not persisted and is lost on daemon restart.

Consequence:

Runtime refs are neither durable mention provenance nor provider-query
authority. Raw fields from these refs must never be copied into the new
reference ledger or an external query.

### Prompt construction and latest-user selection

Repository findings:

- `main.rs` builds system layers, appends plain history, and appends the current
  user message.
- `run_agent_loop` reverse-scans the assembled messages and selects the latest
  `role == "user"` content.
- `prepare_turn_routing` receives only the flag, origin, session ID, latest
  user text, and tool definitions.
- The evidence classifier and routing matrix therefore see no typed
  conversational provenance.

Consequence:

The current SD-card check is latest-message-only by construction. Expanding its
phrase list would not solve the root cause.

### Typed versus legacy routing

Repository findings:

- Absent `BAGENT_EVIDENCE_ORCHESTRATOR` and value `1` enable typed routing.
- Value `0` restores existing legacy routing.
- The current production typed matrix admits supported latest-Mail,
  direct-page, and web-fact intents.
- Targeted/ambiguous Mail, mixed Mail/web, unsupported, unrelated, and
  ordinary agentic turns remain legacy.
- Typed routes clear the legacy tool registry and guidance before execution.
- The SD-card-specific outcome also clears tools and guidance and returns
  before inference.

Consequence:

The universal resolver must live at this shared seam. It may add conservative
no-call outcomes, but it must not broaden typed Mail/web classification or
alter flag-`0` legacy behavior.

### Typed evidence and canonical answers

Repository findings:

- `EvidenceRequest` carries a turn ID, session ID, origin, and non-serialized
  original text.
- Mail evidence carries opaque evidence IDs, connector IDs, body state, and
  body origin.
- Web evidence carries evidence IDs, candidate IDs, requested/final URLs,
  source identity, authority, passages, and citation targets.
- `CanonicalGroundedAnswer` carries canonical text, covered evidence IDs,
  citation targets, source identities, conflicts, shortfalls, and outcome.
- Model polish is validated against canonical invariants and may be rejected,
  unavailable, or skipped.
- These objects are in-memory turn artifacts.
- `ExecOutcome` currently returns only final text, tool-call count, and denied
  approval count.

Consequence:

Strong provenance exists during a typed turn but is discarded before the next
turn. Persisting producer-typed mention artifacts is required. Reparsing final
assistant text cannot restore the lost distinctions.

### Assistant output and transcript sources

Repository findings:

- Swift stores assistant text and rich presentation properties in memory.
- Later history sends only the assistant text.
- Legacy web `source_discovered` events populate in-memory clickable source
  values, but those values are not sent back as typed history.
- Canonical text, accepted polish, rejected polish, legacy model output, and
  Mail-derived assistant output are not durably distinguishable in subsequent
  plain history.

Consequence:

Assistant/model-authored text is not proof of entity identity or disclosure
permission. Rejected polish must contribute no mention. Accepted polish can
contribute a visible mention only when the exact visible span remains mapped
to an existing canonical mention and allowlisted evidence.

### Chat and automation entry points

Repository findings:

- Chat and automation both invoke `agent_exec::run_agent_loop`.
- Automation builds no conversational history.
- An automation uses its stored prompt as the user turn and supplies trusted
  execution-origin metadata separately.
- `outcome_to_status` currently treats any successful, no-denial `ExecOutcome`
  as a completed automation, including a fixed clarification.

Consequence:

The shared execution seam is correct, but `ExecOutcome` needs a typed terminal
disposition before automation can represent reference-blocked results
correctly.

### Diagnostics and other local records

Repository findings:

- Evidence diagnostics use a sanitizer and bounded on-disk retention.
- The sanitizer intentionally excludes evidence bodies and many raw fields,
  but permits fields unrelated to the stricter reference-resolution contract.
- General prompt-debug records retain redacted prompt and output material.
- The general chat audit writes the current user message.

Consequence:

Prompt-debug data, audits, and diagnostic traces are not provenance sources.
Reference-resolution diagnostics need a separate, stricter sanitizer.
Structured confirmation data must not enter model messages, prompt-debug
records, or the general chat audit payload.

## Privacy-safe local characterization

The latest relevant local conversation was characterized read-only without
printing or retaining its text.

Only the following were computed:

- role sequence;
- character counts;
- timestamp;
- SHA-256 digests; and
- structural prompt-layer metadata.

The characterization found one generic-reference case with alternating
user/assistant history and a final user reference turn. The current database
contained zero persisted `chat_turns`. No product identity, sender, subject,
Mail content, URL, connector ID, attachment content, or personal data was
printed, copied into this document, or written elsewhere.

This characterization demonstrates behavior only. Its hashes are not
provenance and are deliberately omitted from this artifact.

## Proven root cause

The root cause is not incomplete phrase matching. It is the absence of a
durable typed chain connecting:

- a visible mention;
- who authored or produced it;
- the turn that introduced it;
- its Mail/web/attachment lineage;
- canonical versus model-produced text;
- its sensitivity and external visibility;
- the current reference expression; and
- explicit authority to disclose a normalized query.

The SD-card workaround prevents one known leak by returning a deterministic
clarification. It cannot safely resolve general references because routing
receives only the latest plain user string.

## Trust and privacy model

### Core rules

1. Provenance is assigned only by the daemon or a typed evidence producer.
2. Client-supplied history can never assign or upgrade provenance.
3. Provenance is immutable. Derived state may remain equally restrictive or
   become more restrictive, never less.
4. Sensitivity and visibility are independent. A public-looking identifier can
   still be sensitive or local-only.
5. A provider query requires both successful reference resolution and explicit
   provider-query authorization.
6. Assistant text is never authority.
7. Evidence content is untrusted data and cannot authorize itself.
8. Local-model output cannot invent, rewrite, upgrade, authorize, or emit a
   provider query.
9. Historical text without the new provenance records is `Unknown`.
10. A sensitive/private identifier is denied even if the user replies with a
    generic confirmation.

### Data that must never silently become a provider query

- Mail bodies;
- Mail subjects;
- senders or recipients;
- Mail rowids or connector IDs;
- attachment contents or attachment IDs;
- private or local-network URLs;
- serial numbers;
- order IDs;
- tracking numbers;
- credentials, tokens, or secrets;
- assistant/model-generated identities;
- prompt-debug or audit content; and
- any provenance-lost historical text.

### Safe reuse categories

A prior mention can be reused directly only when it is:

- daemon-stamped `UserAuthored`;
- deterministically provider-safe and non-sensitive;
- in the same session;
- within both the 10-turn and 30-minute windows;
- compatible with the current expression; and
- the only materially compatible candidate.

A canonical web mention can be reused directly only when:

- its exact visible span is anchored to canonical output;
- its covered evidence ID remains intact;
- its source identity is allowlisted for this purpose;
- its source URL is public HTTP(S) and passed network-safety validation;
- the mention is present in canonical output, not only polish; and
- the result is unambiguous.

## Considered alternatives

### A. Latest-message-only deterministic clarification

Advantages:

- deterministic;
- no schema change;
- no model or provider call;
- simple rollback; and
- low leakage risk.

Disadvantages:

- cannot reuse prior user-authored public entities;
- cannot distinguish assistant, Mail, web, and attachment origins;
- cannot survive restart;
- cannot support general pronouns, generic noun phrases, aliases, or renames;
- requires an ever-growing phrase list; and
- does not solve query authorization.

Verdict: retain only until the universal design passes acceptance.

### B. Scan prior user-authored messages

Advantages:

- low latency;
- good UX for simple follow-ups; and
- deterministic when authorship is proven.

Disadvantages:

- current history is client-supplied plain role/text;
- no durable server authorship or turn identity exists;
- text scanning cannot recover evidence lineage;
- two compatible entities remain ambiguous; and
- restart loses the usable history.

Verdict: use only over newly persisted daemon-stamped `UserAuthored` mentions,
never current plain history or historical `chat_turns`.

### C. Persist typed entity mentions with provenance

Advantages:

- durable authorship and evidence lineage;
- deterministic testing;
- restart support;
- session-window enforcement; and
- compatibility with chat and automation.

Disadvantages:

- additive schema and retention policy;
- producer integration is required; and
- naively storing private mention text creates a privacy risk.

Verdict: required foundation, combined with opaque/restricted text forms.

### D. Persist opaque referents and canonical anchors

Advantages:

- preserves evidence lineage without raw private text;
- supports generic follow-ups to Mail/attachment-derived concepts;
- prevents accidental disclosure; and
- allows canonical web mentions to retain trustworthy mappings.

Disadvantages:

- an opaque private mention may not contain enough text to propose a query
  after client context is lost;
- users may need to type a public term; and
- visible-span anchoring must be producer-owned.

Verdict: required for private and untrusted origins.

### E. Local-model reference resolution

Advantages:

- can order semantically plausible typed candidates; and
- may improve clarification presentation.

Disadvantages:

- prompt-injection risk from evidence-derived text;
- nondeterministic latency and availability;
- potential invented or altered entities;
- unsafe if correctness-critical; and
- unnecessary for deterministic authorization.

Verdict: optional ordering only, disabled by default. It receives existing
candidate IDs and can return only an ordering of those IDs.

### F. Confirm a proposed public search term

Advantages:

- explicit disclosure permission;
- works for assistant, Mail, attachment, and unknown origins;
- deterministic and testable; and
- does not trust model output.

Disadvantages:

- adds a turn;
- cannot pause an automation; and
- is unnecessary for an exact current user-supplied public term.

Verdict: mandatory fallback for non-user/untrusted provenance, not the sole
resolution strategy.

### G. Hybrid resolver

The selected approach combines C and D, the safe subset of B, F for
non-user/untrusted sources, and an optional constrained subset of E.

It has the highest implementation and migration cost, but it is the only
approach that supports all required provenance classes, deterministic
authorization, restart behavior, chat/automation parity, and privacy-safe
provider admission without making model output correctness-critical.

## Selected deep module and seam

Place `ConversationalReferenceResolver` immediately before the current
`routed_evidence_turn`/legacy fallback decision in the shared execution path.

```rust
#[async_trait]
pub(crate) trait ConversationalReferenceResolver {
    async fn resolve_turn(
        &self,
        input: ResolveTurn,
    ) -> Result<ReferenceRoutingDecision, ResolverFault>;

    async fn record_completed_turn(
        &self,
        artifacts: CompletedTurnArtifacts,
    ) -> Result<(), ResolverFault>;

    async fn admit_provider_query(
        &self,
        permit: ProviderQueryPermit,
        operation: ProviderOperation,
    ) -> ProviderQueryAuthorization;
}
```

These three entries are intentional:

- Without `resolve_turn`, routing, privacy, and ambiguity logic would spread
  across chat and automation.
- Without `record_completed_turn`, producers would either lose provenance or
  duplicate mention persistence.
- Without `admit_provider_query`, a returned query would be replayable and the
  network seam would still accept raw strings.

The module owns:

- deterministic span enumeration;
- reference-expression parsing;
- mention eligibility and alias clustering;
- immutable provenance;
- sensitivity/visibility policy;
- session/context windows;
- candidate loading and filtering;
- optional candidate ordering;
- confirmation issuance and consumption;
- deterministic query composition;
- authorization creation and operation admission;
- persistence and retention; and
- structural diagnostic projection.

### Dependency classification and adapters

- Extraction, normalization, policy, query composition, and ambiguity rules
  are in-process dependencies.
- SQLite persistence is local-substitutable:
  - production SQLite adapter;
  - transaction-capable in-memory test adapter.
- Time is an internal seam:
  - system clock adapter;
  - fake clock adapter.
- Optional ranking is an internal seam:
  - production no-op adapter by default;
  - constrained local BaseRT adapter only in a future enabled stage;
  - scripted fake adapter for tests.
- The provider is a true external dependency:
  - production typed web adapter;
  - counting/panicking test adapter.

Internal adapters do not expand the resolver's external interface.

## Typed domain model

Names may change mechanically during implementation, but the expressed facts
and invariants may not.

```rust
pub(crate) struct MentionId(Uuid);

pub(crate) enum MentionText {
    PublicVisible {
        display: String,
        normalized: NormalizedPublicText,
    },
    Restricted {
        span_hmac: MentionDigest,
        kind_hint: EntityKind,
    },
    Opaque {
        fingerprint: MentionDigest,
        kind_hint: EntityKind,
    },
}

pub(crate) struct ConversationMention {
    id: MentionId,
    session_id: SessionId,
    introduced_turn_id: TurnId,
    entity_kind: EntityKind,
    text: MentionText,
    provenance: MentionProvenance,
    visibility: MentionVisibility,
    sensitivity: MentionSensitivity,
    directly_user_supplied: bool,
    untrusted_evidence: bool,
    anchor: MentionAnchor,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

pub(crate) enum MentionProvenance {
    UserAuthored,
    AssistantAuthored {
        lineage: AssistantLineage,
    },
    MailEvidence {
        evidence_id: EvidenceId,
        body_origin: BodyOrigin,
    },
    WebEvidence {
        evidence_id: EvidenceId,
        source_identity: SourceIdentity,
        authority: SourceAuthority,
        public_source: PublicUrlReference,
        canonical: bool,
    },
    AttachmentEvidence {
        attachment_ref: OpaqueAttachmentReference,
    },
    Unknown,
}

pub(crate) enum AssistantLineage {
    Canonical,
    AcceptedPolish,
    RejectedPolish,
    Legacy,
    Unknown,
}

pub(crate) enum MentionVisibility {
    ProviderSafe,
    LocalOnly,
    ConfirmationOnly,
    Unknown,
}

pub(crate) enum MentionSensitivity {
    Public,
    Private,
    Sensitive,
    Unknown,
}

pub(crate) enum EntityKind {
    Person,
    Organization,
    Place,
    Product,
    TechnicalStandard,
    DocumentTitle,
    PublicUrl,
    Unknown,
}

pub(crate) struct ReferenceExpression {
    kind: ReferenceExpressionKind,
    span: CurrentTurnSpan,
    compatible_kinds: Vec<EntityKind>,
    grammatical_number: GrammaticalNumber,
}

pub(crate) enum ReferenceExpressionKind {
    Pronoun,
    Demonstrative,
    GenericNoun,
    NamedReuse,
    Comparison,
}

pub(crate) struct ReferenceCandidate {
    mention_id: MentionId,
    compatibility: CandidateCompatibility,
    recency: CandidateRecency,
    eligibility: CandidateEligibility,
    denial_reasons: Vec<ResolutionReason>,
}

pub(crate) enum ResolutionConfidence {
    ExactCurrent,
    UniqueRecent,
    Confirmed,
    Ambiguous,
}

pub(crate) enum ReferenceResolution {
    ResolvedUserPublic {
        mention_id: MentionId,
        confidence: ResolutionConfidence,
        permit: ProviderQueryPermit,
    },
    ResolvedConfirmedPublic {
        mention_id: MentionId,
        confidence: ResolutionConfidence,
        permit: ProviderQueryPermit,
    },
    Ambiguous {
        clarification: ClarificationRequest,
    },
    MissingReferent {
        clarification: ClarificationRequest,
    },
    ConfirmationRequired {
        clarification: ClarificationRequest,
    },
    PrivateSourceDenied {
        reason: ResolutionReason,
        clarification: ClarificationRequest,
    },
    Expired {
        clarification: ClarificationRequest,
    },
    Unsupported {
        reason: ResolutionReason,
        clarification: ClarificationRequest,
    },
    RollbackLegacy,
}

pub(crate) enum ProviderQueryAuthorization {
    Authorized(AuthorizedWebQuery),
    Denied {
        reason: AuthorizationDenial,
    },
}

pub(crate) struct ClarificationRequest {
    challenge_id: Option<ConfirmationId>,
    safe_entity_label: Option<EntityKind>,
    local_display_proposal: Option<ProposedPublicTerm>,
    expires_at: Option<DateTime<Utc>>,
    allowed_actions: Vec<ClarificationAction>,
    normalized_reason: ResolutionReason,
}
```

`MentionText`, `ProviderQueryPermit`, `AuthorizedWebQuery`, confirmation
ciphertext, and proposal-bearing clarification values must implement custom
redacted `Debug`. Query-bearing types must not derive `Serialize`.

`ResolutionConfidence` is a deterministic band, not a probability.

### Producer-owned completed-turn artifacts

```rust
pub(crate) struct CompletedTurnArtifacts {
    session_id: SessionId,
    turn_id: TurnId,
    origin: EvidenceOrigin,
    output: ProvenancedOutput,
}

pub(crate) enum ProvenancedOutput {
    CanonicalWeb(CanonicalWebArtifacts),
    TypedMail(TypedMailArtifacts),
    TypedAttachment(TypedAttachmentArtifacts),
    Assistant(AssistantArtifacts),
    NoMentions,
}
```

Callers cannot pass a free-form provenance enum. Each variant's constructor is
owned by the producer that has the necessary evidence:

- canonical web construction owns canonical mention/evidence/source mappings;
- typed Mail owns opaque evidence/body-origin mappings;
- attachment ingestion owns opaque attachment mappings; and
- legacy assistant output can create only assistant/unknown restricted
  mentions.

Rejected or unavailable model polish produces status only and no mention.

## Deterministic resolution algorithm

### 1. Rollback override

If `BAGENT_EVIDENCE_ORCHESTRATOR=0`:

- return `RollbackLegacy`;
- do not read or write resolver tables;
- do not issue or consume confirmation;
- do not call a ranker;
- do not emit resolver diagnostics; and
- execute the existing legacy route unchanged.

This check occurs before all other resolver work.

### 2. Daemon-stamped turn

Assign a new daemon-generated `TurnId` at request ingress for both chat and
automation. The current chat message and saved automation prompt are the only
texts that can be stamped `UserAuthored` at ingress.

Client history remains model context with `Unknown` resolver provenance.

### 3. Original-scope protection

Classify the untouched original request structurally before resolving
candidates:

- supported Stage 9 typed intents remain supported;
- ordinary unrelated requests return `NotApplicable` and retain existing
  behavior;
- non-reference mixed Mail/web remains on the existing legacy matrix;
- a reference-bearing mixed Mail/web request returns a terminal safe
  clarification instead of falling into legacy provider use; and
- a detected unsafe reference-bearing web request cannot fall through to the
  legacy model/tool path.

### 4. Current-message span enumeration

Enumerate candidates as exact spans of current user text:

- one public HTTP(S) URL;
- quoted or backticked spans;
- Unicode capitalized name spans;
- make/model spans containing a public make plus model token;
- standards matching controlled prefixes such as ISO, IEC, IEEE, or RFC plus
  a public standard number;
- explicit document-title spans; and
- alias relations using user-authored terms such as "formerly", "now called",
  "renamed", "also called", or "aka".

The enumerator may generate multiple bounded contiguous spans. It does not
decide authorization.

An optional local extractor may return only IDs of these exact spans and an
entity-kind suggestion. Unknown IDs, changed text, invented text, duplicated
IDs, or out-of-range offsets invalidate the whole optional result. The
deterministic extractor remains authoritative.

### 5. Reference-expression parsing

Recognize:

- bare pronouns such as "it" and "its";
- demonstratives such as "that" and "the one above";
- generic noun phrases such as "that product", "the company", "that SD card",
  and "the medication";
- named reuse and partial aliases; and
- comparison expressions.

The expression produces compatible entity kinds and grammatical constraints,
not a provider query.

### 6. Sensitivity and network policy

Apply sensitivity before recency or semantic ranking.

Always deny:

- credentials and tokens;
- serial, order, and tracking identifiers;
- Mail/connector identifiers;
- private/local/loopback/link-local URLs or hosts;
- unknown long identifier-like values; and
- a term whose safe public projection cannot be separated from private data.

Public HTTP(S) URLs must pass the existing redirect and network-safety policy.

### 7. Current explicit entity

An exact current-turn `UserAuthored` public entity wins when:

- the request explicitly asks for supported web work;
- the span passes sensitivity checks;
- its public term is non-empty after normalization; and
- the original request is not mixed/unsupported.

This creates `ResolvedUserPublic`.

### 8. Candidate window

Otherwise load mentions satisfying every condition:

- same session;
- introduced in the last 10 user-visible turns;
- introduced no more than 30 minutes ago;
- compatible entity kind or a user-authored alias cluster; and
- not deleted or expired.

Both limits apply; reaching either limit removes the mention from eligibility.

### 9. Eligibility by provenance

- `UserAuthored` public/provider-safe: eligible for direct reuse.
- canonical `WebEvidence`: eligible only with intact allowlisted provenance.
- `AssistantAuthored`, `MailEvidence`, `AttachmentEvidence`, and `Unknown`:
  confirmation required.
- private/sensitive: denied.
- rejected polish: not a candidate.
- accepted polish span absent from canonical output: assistant-derived and
  confirmation-required, never canonical.

### 10. Ambiguity

Resolve only when exactly one materially compatible eligible mention or alias
cluster remains.

If two or more candidates remain, return `Ambiguous`. Recency alone cannot
silently select between two materially compatible user-authored entities.

The optional local ranker can order candidates for a clarification UI only.
It cannot change `Ambiguous` to a resolved outcome.

### 11. Query composition

Do not replace reference text and re-run the current classifier.

Use:

```rust
pub(crate) enum ResolvedRequestView {
    LiteralCurrentTurn,
    WebReference {
        original_request: UserAuthoredText,
        reference_expression: ReferenceExpression,
        authorized_mention: MentionId,
    },
}
```

The deterministic composer combines:

- operation: research, specifications, compare, verify, or lookup;
- the exact authorized public term;
- approved modifiers such as current/latest; and
- no arbitrary transcript or evidence text.

Diversification queries must be precomputed and authorized. They may add only
controlled public discovery modifiers and may not add another entity,
identifier, private token, or evidence-derived text.

### 12. Sealed authorization

Mint `ProviderQueryPermit` bound to:

- session ID;
- initiating reference turn ID;
- executing confirmation/current turn ID;
- normalized query-plan HMAC;
- authorization method;
- provider scope;
- exact operation budget;
- issue and expiry times; and
- a fresh non-replayable nonce.

Only `admit_provider_query` can reveal `AuthorizedWebQuery`.

### 13. Provider admission

Immediately before each provider network operation:

- verify the flag is still enabled;
- verify session and both turn bindings;
- verify expiry;
- verify the operation belongs to the sealed plan;
- atomically reserve the allowed attempt; and
- deny replays or out-of-plan variants.

Confirmation one-use means one authorized evidence turn, not an unbounded
query. The turn can use only its sealed primary/diversification plan and
existing bounded retries.

## Provider-query authorization table

| Selected source | Resolution | Typed web | Provider call |
|---|---|---:|---:|
| Explicit current user public entity | `ResolvedUserPublic` | Yes | Authorized |
| Explicit current public HTTP(S) URL | `ResolvedUserPublic` after network-safety validation | Yes | Authorized |
| Unique prior user public entity in window | `ResolvedUserPublic` | Yes | Authorized |
| Exact confirmed public term | `ResolvedConfirmedPublic` | Yes | Authorized once |
| Canonical web mention with intact allowlisted mapping | `ResolvedUserPublic` | Yes | Authorized |
| Accepted polish term absent from canonical output | `ConfirmationRequired` | No | No |
| Rejected/unavailable polish term | `MissingReferent` or another candidate | No | No |
| Ordinary assistant mention | `ConfirmationRequired` | No | No |
| Typed Mail mention | `ConfirmationRequired` | No | No |
| Assistant paraphrase of Mail | `ConfirmationRequired` | No | No |
| Attachment mention | `ConfirmationRequired` | No | No |
| Historical plain text | `ConfirmationRequired` or `MissingReferent` | No | No |
| Private/sensitive identifier | `PrivateSourceDenied` | No | No |
| Private/local URL | `PrivateSourceDenied` | No | No |
| Multiple compatible entities | `Ambiguous` | No | No |
| Missing referent | `MissingReferent` | No | No |
| Reference-bearing mixed Mail/web | `Unsupported` clarification | No | No |
| Automation requiring interaction | typed blocked | No | No |
| Flag `0` | `RollbackLegacy` | No resolver route | Existing legacy behavior |

## Confirmation protocol

### Wire input

Add a structured field to chat ingress:

```rust
pub(crate) struct ConfirmationEnvelope {
    challenge_id: ConfirmationId,
    proposed_term: String,
}
```

Do not parse "yes", "confirm", or similar natural-language text as
confirmation.

### Challenge binding

Each challenge binds:

- one session;
- one initiating reference turn;
- one selected mention or opaque referent;
- one normalized proposed-query HMAC;
- one five-minute expiration; and
- one use.

### Proposal storage

The exact proposed term:

- is displayed only in the local chat UI/SSE event;
- never enters diagnostics, prompt construction, the general chat audit, or
  prompt-debug records;
- is stored for at most five minutes as authenticated ciphertext;
- is accompanied by a keyed HMAC for equality checks; and
- is deleted after expiry/consumption, leaving only a bounded structural
  replay tombstone.

A dedicated local encryption key is created only on first enabled resolver use,
not by the additive schema migration. The exact Keychain service/account names
must be fixed in implementation design review before code is authorized.

### Confirm

On confirmation:

1. normalize the submitted exact term;
2. match its keyed HMAC;
3. verify session and initiating-turn bindings;
4. verify expiry and unused state;
5. re-run sensitivity checks;
6. atomically consume the challenge and create one query authorization; and
7. execute only the sealed evidence turn.

### Edit

Editing the proposed term:

- invalidates the pending challenge;
- creates a new current-turn `UserAuthored` mention;
- requires an explicit web request or explicit UI action; and
- is evaluated by the ordinary current-user path.

### Unsafe or unavailable proposal

If an opaque private referent cannot be projected into an exact safe public
term, do not display or persist a guessed term. Ask the user to type a public
make/model, public entity, or public URL. That reply becomes a new
`UserAuthored` mention.

### Failure cases

Cross-session, wrong-turn, expired, reused, replayed, malformed, or
normalization-mismatched confirmation produces `Expired` or
`ConfirmationRequired`, with zero provider and zero legacy-model calls.

## Persistence and schema design

Use an additive migration. V15 was the next apparent number at the document
baseline, but implementation must refresh HEAD and select the next free
ordinal.

### `reference_turns`

Structural fields:

- `id` primary key;
- `session_id`;
- `origin`;
- `producer_class`;
- monotonic session sequence;
- `created_at`; and
- `expires_at`.

No conversation content.

### `conversation_mentions`

Fields:

- mention/session/turn IDs;
- entity kind;
- provenance and assistant-lineage class;
- visibility and sensitivity;
- direct-user and untrusted-evidence booleans;
- public display/normalized text, nullable;
- restricted span HMAC or opaque fingerprint, nullable;
- opaque evidence ID, nullable;
- public source identity/URL/authority, nullable and allowed only for canonical
  public web provenance;
- Mail body-origin class, nullable;
- creation/expiry/deletion timestamps.

Database constraints must enforce:

- public text only for provider-safe public mentions;
- restricted/opaque mentions cannot contain public/raw text;
- no connector IDs or Mail rowids;
- no raw Mail or attachment content;
- no private URL in public URL columns; and
- coherent provenance-specific columns.

### `mention_anchors`

Fields:

- turn ID;
- mention ID;
- exact visible start/end offsets;
- visible-span HMAC; and
- canonical/assistant display class.

Client-provided offsets cannot create provenance. Producer-owned artifacts
create the anchor; the client can only round-trip its opaque ID.

### `reference_confirmations`

Fields:

- confirmation/session/initiating-turn IDs;
- mention ID, nullable;
- encrypted proposed term;
- normalized-query HMAC;
- creation/expiry/consumption timestamps;
- consuming turn ID; and
- state.

No raw query.

### `query_authorizations`

Fields:

- authorization/session/initiating/execution turn IDs;
- confirmation ID, nullable;
- query-plan HMAC;
- authorization method;
- issued/expiry/terminal timestamps;
- bounded attempt counters; and
- structural state.

Plaintext authorized queries exist only inside the non-serializable in-memory
capability for the active turn.

### Historical behavior

- No backfill from `chat_turns`, prompt-debug traces, audits, or client
  history.
- Existing historical assistant text is `Unknown`.
- Existing sessions contain zero trusted prior mentions until new turns create
  producer-owned records.
- The startup legacy purge remains disabled from touching only the new
  reference tables; legacy chat persistence is not re-enabled.

### Retention and deletion

- Mention eligibility/storage: last 10 user-visible turns and at most 30
  minutes.
- Delete expired mentions and anchors.
- Delete all session mentions/challenges/authorizations on session deletion.
- Confirmation proposal: five minutes.
- Consumed/expired challenge and authorization tombstones: structural HMAC
  state for 24 hours, then delete.
- Restricted evidence text: never stored raw.
- Resolver diagnostics: same bounded seven-day evidence-diagnostics framework,
  with the stricter event sanitizer below.

## Diagnostics contract

Add one `reference_resolution` event family with its own sanitizer.

Allowed fields:

- `outcome`;
- `provenance_class`;
- `candidate_count`;
- `ambiguity_state`;
- `authorization_decision`;
- `normalized_reason`;
- `origin`;
- `duration_ms`; and
- structural terminal status.

Forbidden fields:

- mention text;
- proposed term;
- query text;
- unkeyed query/mention hashes;
- conversation content;
- evidence content;
- evidence IDs;
- source identities or URLs;
- source domains;
- Mail subject/sender/recipient/body;
- rowids or connector IDs;
- attachment identifiers/content;
- model prompts/output; and
- credentials/secrets.

Do not reuse the broader evidence event allowlist. Tests must inject synthetic
forbidden values into every potential field and prove they are absent from
both stored and exported traces.

## Outcome matrix

### `ResolvedUserPublic`

- Typed web routing: yes.
- Provider: allowed only through sealed authorization.
- Chat: normal canonical grounded answer.
- Diagnostics: user/canonical-web provenance class, candidate count,
  authorized decision, normalized reason, timing.
- Automation: proceeds only when its saved prompt contains an explicit safe
  public term; it has no conversational reuse.

### `ResolvedConfirmedPublic`

- Typed web routing: yes.
- Provider: allowed for one sealed evidence turn.
- Chat: normal canonical grounded answer.
- Diagnostics: confirmed class and structural authorization.
- Automation: cannot originate an interactive confirmation.

### `Ambiguous`

- Typed web routing: no.
- Provider: no.
- Legacy model/tools: no.
- Chat: ask the user to name the exact public term or URL; do not expose
  private candidate text.
- Diagnostics: candidate count, ambiguous state, denied authorization.
- Automation: typed `ReferenceBlocked`.

### `MissingReferent`

- Typed web routing: no.
- Provider: no.
- Legacy model/tools: no.
- Chat: ask for the exact public entity or URL.
- Diagnostics: zero candidates and normalized missing reason.
- Automation: typed `ReferenceBlocked`.

### `ConfirmationRequired`

- Typed web routing: no until a later valid confirmation turn.
- Provider: no.
- Legacy model/tools: no.
- Chat: structured Confirm/Edit UI when a safe proposal exists; otherwise ask
  the user to type a public term.
- Diagnostics: provenance class and confirmation-required decision only.
- Automation: typed `ReferenceBlocked`; proposal text is not persisted in run
  summary.

### `PrivateSourceDenied`

- Typed web routing: no.
- Provider: no.
- Legacy model/tools: no.
- Chat: explain that a private/sensitive identifier cannot be sent and ask for
  a public term.
- Diagnostics: denied authorization and normalized sensitivity reason.
- Automation: typed `ReferenceBlocked`.

### `Expired`

- Typed web routing: no.
- Provider: no.
- Legacy model/tools: no.
- Chat: ask to reconfirm or restate the public term.
- Diagnostics: expired normalized reason.
- Automation: typed `ReferenceBlocked`.

### `Unsupported`

- Typed web routing: no.
- Provider: no.
- Legacy model/tools: no for a detected unsafe reference-bearing request.
- Chat: fixed safe clarification.
- Diagnostics: unsupported normalized reason.
- Automation: typed `ReferenceBlocked`.

Ordinary unrelated requests are `NotApplicable`, not `Unsupported`, and retain
existing routing.

### `RollbackLegacy`

- Resolver typed routing: no.
- Resolver provider admission: no.
- User output: existing legacy behavior.
- Resolver diagnostics: none.
- Automation: existing behavior.
- Migration rollback: not required.

## Chat and automation behavior

### Shared decision

Chat and automation call the same resolver from the same execution seam.
Neither caller may independently infer reference provenance or provider
authority.

### Chat

Chat may:

- emit the local structured confirmation event;
- receive a structured confirmation envelope;
- edit a proposed term into a new user-authored request; and
- continue to a canonical typed web answer after authorization.

### Automation

Automation may not pause for confirmation.

For `Ambiguous`, `MissingReferent`, `ConfirmationRequired`,
`PrivateSourceDenied`, `Expired`, or `Unsupported`:

- return a typed `ReferenceBlocked` completion;
- persist a fixed structural summary with no candidate/query text;
- make zero provider calls;
- make zero Mail/connector calls;
- make zero legacy-model calls; and
- emit the same structural resolution event used by chat.

Extend the execution result:

```rust
pub(crate) enum TurnCompletion {
    Completed,
    Partial,
    ReferenceBlocked(ReferenceOutcomeCode),
}

pub(crate) struct ExecOutcome {
    final_text: String,
    tool_calls_used: usize,
    approvals_denied: usize,
    completion: TurnCompletion,
    completed_artifacts: CompletedTurnArtifacts,
}
```

Automation maps `ReferenceBlocked` to a typed blocked status. If an additive
automation status cannot be introduced in the same accepted migration stage,
use `Failed` with a normalized structural reason temporarily; never report it
as completed.

## User-visible clarification templates

Templates contain no private candidate identity unless a safe local exact
proposal is intentionally shown:

- Ambiguous: "I found more than one possible referent. Please provide the
  exact public name, make/model, or URL to research."
- Missing: "I could not determine what the reference points to. Please provide
  the exact public name, make/model, or URL."
- Confirmation: "Search the web for “{exact public proposal}”?" with structured
  Confirm and Edit actions.
- Private denied: "I cannot send a private identifier or private-source text
  to a web provider. Please provide a public name, make/model, or URL."
- Expired: "That confirmation expired. Please confirm or provide the public
  term again."
- Automation blocked: "Blocked: this web reference requires an explicit public
  term or interactive confirmation."

Localized wording may be added later without deriving route state from the
rendered string.

## Synthetic acceptance matrix

All fixtures use synthetic entities and content.

| Case | Expected result | Provider/model behavior |
|---|---|---|
| Current message explicitly names `Acme Atlas X2` and asks for current specifications | `ResolvedUserPublic` | Typed web allowed; no legacy model |
| Earlier user message names one public product; current says "look up its specifications" | `ResolvedUserPublic` | Typed web allowed |
| Two earlier user entities; current says "compare it" | `Ambiguous` | Zero provider and zero model calls |
| Referent exists only in ordinary assistant output | `ConfirmationRequired` | Zero provider and zero legacy model |
| Referent exists only in typed Mail evidence | `ConfirmationRequired` or request user-entered public term | Zero provider and zero legacy model |
| Assistant paraphrases a Mail-derived identity | `ConfirmationRequired` | Zero provider and zero legacy model |
| User confirms exact proposed public make/model | `ResolvedConfirmedPublic` | One sealed evidence turn |
| User edits proposed term | New `UserAuthored` mention; old challenge invalid | Only new explicit request can authorize |
| Confirmation reused or replayed | `Expired`/denied | Zero provider |
| Confirmation used after five minutes | `Expired` | Zero provider |
| Confirmation used in another session | denied | Zero provider |
| Prior canonical web answer contains cited public entity with intact mapping | Eligible unique candidate | Typed web allowed |
| Entity appears only in rejected polish | No candidate from polish | Zero influence on result |
| Attachment contains product name | `ConfirmationRequired` | Zero provider |
| Attachment contains serial number | `PrivateSourceDenied` | Zero provider |
| Public HTTP(S) URL | `ResolvedUserPublic` after network checks | Direct typed web allowed |
| Private/local URL | `PrivateSourceDenied` | Zero provider |
| Person, organization, place, product, standard, document title | Exact synthetic span types | Deterministic eligibility |
| Bare "it", "that", "the one above" with one compatible mention | Unique resolution according to provenance | As authorized by provenance |
| Generic "that SD card", "the company", "the medication" | Type-compatible resolution or clarification | No unsafe fallback |
| User says an entity is now called a new name | User-authored alias cluster | Newest explicit public alias used |
| Mixed Mail plus web request with reference | `Unsupported` clarification | Zero provider/model/tools |
| Later follow-up after completed Mail turn | `ConfirmationRequired` | Zero provider/model |
| Same fixture in chat and automation | Same resolution code | Automation blocks interaction |
| `BAGENT_EVIDENCE_ORCHESTRATOR=0` | `RollbackLegacy` | Existing legacy behavior |
| Daemon/app restart within mention window | Typed public provenance survives | Same deterministic result |
| Restart with pending confirmation | Binding survives; exact/replay rules preserved | No premature provider call |
| Diagnostic/export scan | Structural fields only | No forbidden content |
| Any blocked outcome | terminal response | Zero provider and zero legacy-model calls |

## Testing strategy

### Pure resolver contract

Type: unit.

Public seam: `resolve_turn`.

Fixtures:

- synthetic current messages;
- typed mention ledger;
- fake clock;
- deterministic IDs;
- no-op and malicious ranker adapters.

Red-capable assertions:

- every outcome above;
- exact 10-turn/30-minute cutoffs;
- ambiguity and alias behavior;
- sensitivity precedence;
- historical `Unknown`;
- model unavailability/malformed output invariance; and
- flag-`0` zero repository/ranker interaction.

Forbidden:

- provider calls;
- legacy model calls;
- raw Mail/attachment data; and
- caller-assigned provenance.

### Persisted provenance round trip

Type: SQLite integration.

Public seam: resolver with production repository adapter.

Fixtures:

- synthetic user, assistant, Mail, web, and attachment artifacts;
- canonical and rejected polish artifacts;
- fake timestamps; and
- encrypted confirmation proposal.

Expected:

- immutable provenance round trip;
- opaque evidence linkage;
- restart/reopen behavior;
- alias and expiry behavior;
- consumed confirmation replay prevention; and
- session deletion cascade.

Forbidden:

- forbidden raw values in any table;
- backfill from historical plain text; and
- public visibility for restricted provenance.

### Shared chat/automation routing

Type: integration.

Public seam: shared routing decision in `run_agent_loop`.

Expected:

- identical resolution codes;
- interactive-required automation becomes blocked;
- terminal results expose no legacy tools/guidance;
- ordinary unrelated turns preserve current behavior; and
- reference-bearing unsafe turns do not fall through.

### Confirmation authorization

Type: integration.

Public seams:

- `resolve_turn`;
- `admit_provider_query`.

Cases:

- exact confirmation;
- edit;
- wrong session;
- wrong initiating turn;
- expiry;
- replay;
- concurrent double consume;
- query mismatch;
- out-of-plan diversification; and
- sensitive term on revalidation.

Expected:

- one atomic winner where applicable;
- all denied cases leave provider count zero.

### Provider-call admission

Type: integration and compile-time interface proof.

Change the typed provider adapter so a raw `&str` cannot be passed to its
network-search entry. Use:

- counting adapter;
- adapter that panics on any unexpected call; and
- operation trace.

Prove:

- no capability means no provider call;
- wrong/expired/replayed capability means no provider call;
- each admitted call belongs to the sealed plan; and
- bounded retry counts cannot be exceeded.

### SSE terminal behavior

Type: integration.

Expected:

- one `reference_resolution` terminal event;
- fixed/local presentation as appropriate;
- normal outer `done`;
- no evidence-acquisition event for blocked outcomes; and
- no proposal/query/private text in diagnostics or exports.

### Model-independence proof

Type: unit and integration.

Use local ranker outputs containing:

- unknown IDs;
- duplicate IDs;
- reordered valid IDs;
- altered text;
- invented entities;
- malformed output;
- timeout; and
- unavailability.

Only valid ordering of existing IDs may affect clarification display order.
No output may change provenance, sensitivity, ambiguity, authorization, or
query text. Rejected/unavailable ranker output must match the no-ranker
resolution exactly.

### Signed-app session restart

Type: signed E2E.

Prove:

- changed daemon PID after restart;
- same-session public mention recovery within the window;
- historical provenance does not upgrade;
- pending confirmation retains exact binding;
- consumed confirmation cannot replay;
- expired mention/challenge fails closed;
- blocked cases generate zero Tavily and BaseRT requests; and
- private content does not appear in diagnostics/export.

### Rollback

Type: signed E2E.

Extend the existing Stage 9 rollback acceptance:

- capture protected state before activation;
- set `BAGENT_EVIDENCE_ORCHESTRATOR=0`;
- restart and prove changed PID;
- assert zero resolver reads/writes/events;
- prove existing legacy behavior;
- compare protected table/file/credential/Keychain hashes; and
- restore absent production default only after the campaign.

## Proving zero provider and legacy-model calls

Use all of the following:

1. Typed provider interface requires `AuthorizedWebQuery`.
2. Blocked routing returns no typed evidence request, tools, or guidance.
3. Counting/panicking provider adapters remain at zero.
4. Counting/panicking inference adapters remain at zero.
5. Operation traces contain no `web.search`, `web.fetch`, Mail, or legacy tool
   operation.
6. Signed acceptance counts Tavily and BaseRT HTTP activity.
7. Automation run outcome is structurally blocked rather than completed.
8. Diagnostic export contains one structural resolver terminal and no
   evidence acquisition for the blocked turn.

The type constraint is the primary proof. Runtime counters and signed
observation verify integration and packaging.

## Proving rejected or unavailable model output cannot influence resolution

- Rejected polish never enters `CompletedTurnArtifacts` as a mention.
- Accepted polish may map only an exact visible span already present in the
  canonical mention set.
- The optional ranker sees candidate IDs, not authority-bearing mutable
  records.
- The ranker cannot emit text or a provider query.
- Unknown/duplicate/malformed IDs invalidate the whole ranker result.
- Ambiguity remains ambiguity regardless of ranking.
- Authorization is created by deterministic policy after model output
  validation.
- Golden tests compare no-model, unavailable-model, rejected-model, and
  malicious-model structural outcomes for equality.

## Migration strategy

The migration is forward-only and additive:

- create new reference tables;
- do not alter or backfill historical `chat_turns`;
- do not restore legacy conversation persistence;
- do not modify existing protected data;
- allow current sessions to continue with zero trusted historical mentions;
- create encryption material only on first enabled resolver use; and
- keep tables dormant when resolver enforcement is disabled.

No down migration is required for rollback. Flag `0` ignores the new tables and
retains existing legacy behavior.

## Feature rollout

Use a subordinate resolver mode overridden by the Stage 9 flag:

1. **Contracts only**
   - domain types;
   - in-memory repository;
   - red unit tests;
   - no production routing.
2. **Persistence**
   - additive migration;
   - producer-owned artifacts;
   - restart/retention tests;
   - no routing changes.
3. **Observe**
   - compute structural outcomes;
   - do not authorize providers;
   - no user-visible behavior change;
   - structural diagnostics only.
4. **Fixture enforcement**
   - enabled only inside deterministic signed acceptance.
5. **Explicit opt-in**
   - enforcement behind an explicit subordinate flag.
6. **Default enablement**
   - only after exact-commit privacy, restart, automation, provider-admission,
     and rollback gates pass twice.
7. **Workaround replacement**
   - remove the SD-card-specific classifier branch only after the universal
     resolver is the accepted default.

`BAGENT_EVIDENCE_ORCHESTRATOR=0` overrides every subordinate mode.

## Rollback plan

Flag `0` must:

- short-circuit before resolver state access;
- emit no resolver events;
- create/consume no confirmation;
- mint/admit no authorization;
- call no optional ranker;
- require no data migration rollback; and
- use the existing legacy router after daemon restart.

The additive tables and optional encryption key may remain dormant. Rollback
must not delete or mutate them because rollback is routing-only.

Acceptance must prove:

- changed PID;
- existing legacy operation behavior;
- zero typed resolver activity;
- zero typed Tavily activity attributable to the resolver;
- protected DB/table/file invariance;
- rules and automation invariance;
- attachment invariance;
- daemon credential invariance; and
- Keychain invariance.

## Acceptance gates before replacing the workaround

All must pass on the same exact clean commit:

1. Pure resolver matrix.
2. SQLite provenance/restart/retention matrix.
3. Chat/automation shared routing.
4. Exact confirmation binding/replay matrix.
5. Compile-time and runtime provider admission.
6. Zero-call blocked outcomes.
7. Rejected/unavailable model invariance.
8. SSE terminal contract.
9. Diagnostic/export privacy scan.
10. Signed synthetic E2E twice with identical structural results.
11. Signed restart behavior.
12. Signed flag-`0` rollback with changed PID and protected-state invariance.
13. Observational live smoke with no unsafe disclosure.
14. Clean runtime teardown.

An apparent positive result with missing provenance, invalid fixture
provenance, unsupported query admission, or unverified signed/runtime evidence
is a failure, not a pass.

## Open blockers

Architectural decisions are complete. Implementation blockers are:

- explicit review and authorization of this specification;
- refresh HEAD and choose the next free migration ordinal;
- fix exact Keychain service/account names for the ephemeral confirmation
  encryption key during implementation review;
- write red tests before production integration;
- implement producer-owned mention artifacts without reparsing text;
- change typed provider interfaces to require sealed authorization;
- add the typed automation-blocked terminal;
- complete the exact-commit signed acceptance gates; and
- preserve concurrent Tavily work and protected untracked files throughout.

The Keychain naming choice is operational naming, not permission to alter the
confirmation semantics or store plaintext.

## Implementation authorization verdict

**NOT AUTHORIZED.**

This document authorizes no resolver implementation and no production change.
The current SD-card-specific workaround must remain in place until the design
is reviewed and implementation is explicitly authorized.

# Stage 8 definitive requalification against 8536f01

Date: 2026-07-30

Revision under test: `8536f0115a50e2c011b39ce31ac79cc16b47d8fc`

Overall verdict: **FAIL**

Stage 9 implementation: **NOT AUTHORIZED**

The remediation is correct under deterministic regression, workspace, release,
privacy, signing, standards, and specification review. The definitive signed
campaign still did not pass every mandatory criterion in the same immutable
revision. Mail body hydration remained environmentally unavailable, and live
web discovery/fetch acquisition did not assemble the authoritative,
corroborated, or conflict bundles required by criteria 2–4.

This report records only structural evidence and public source domains. It
contains no Mail content, private prompt, raw connector identifier, credential,
fetched passage, or model response.

## Preserved correction and remediation commits

- `7af52769f4350959e38a9ac785f3a1af963e273c`
  (`fix: remediate Stage 8 evidence correctness`) preserves the four reviewed
  red-to-green corrections from the original worktree: no fabricated
  authoritative discovery provenance, contradictory office-holder claims
  preserved, explicit office-holder negation preserved, and equivalent numeric
  formats normalized consistently.
- `8536f0115a50e2c011b39ce31ac79cc16b47d8fc`
  (`fix: complete Stage 8 evidence remediation`) contains the separate Mail,
  authority, canonical renderer, and Bratislava safety remediation.

Stage 9 routing or implementation was not enabled.

## Definitive signed criteria

| # | Criterion | Verdict | Exact signed-revision evidence |
| ---: | --- | --- | --- |
| 1 | Exact three-email Mail content flow | **FAIL — environmental** | Turns `27c0895f-7ce4-45e2-ba4c-d0c7ede6490b`, `aa099b8c-9b36-4c93-a9fe-517352b64342`, and `5caa8652-faf3-41a7-93be-c4fd7f223945` each performed one list and three distinct reads. Every read completed with structural origin `unavailable`, `attempt_count=2`, `retries=1`, execution status `succeeded`, and no normalized failure. Each turn ended unavailable at 0/3 with exactly one outcome and one done. The repeatability and successful bounded lifecycle make another Mail code change unjustified without a deterministic lifecycle reproduction. |
| 2 | Relevant authoritative first-party web answer | **FAIL — acquisition** | Turn `04b2da9f-46bc-41d7-9738-e738c50c2b07` requested one authoritative source. Typed discovery produced Wikipedia and archived official-site candidates; five fetches yielded zero eligible first-party claims. Validation entered recovery and the terminal was a safe verification shortfall at 0/1. The remediation privately recognizes the fetched institutional-title/pronoun-biography shape without hostname trust, but that usable current official page was not acquired in this signed turn. |
| 3 | Two independently grounded corroborated claims form a verified terminal | **FAIL — live gate unproven** | Turn `0fb1caf2-eb41-4d69-bf29-98a33b997fbf` acquired one eligible claim from `planetrulers.com`; the remaining fetched public sources were irrelevant, duplicate, or failed. The result safely shortfalled at 1/2. The canonical defect is fixed by red/green complete-bundle regressions, but this signed turn did not form the required complete two-source bundle. |
| 4 | Renderable conflict | **FAIL — acquisition** | Turn `7af48d1c-9739-4946-a0a7-1ee3c5bf9418` acquired one eligible Mount Everest claim from `en.wikipedia.org`; the other public fetches were irrelevant, duplicate, or failed. It safely shortfalled at 1/2 with no conflict. Deterministic conflict, explicit-negation, date/definition adjacency, and equivalent-number regressions pass, but the signed live bundle was incomplete. |
| 5 | Bratislava ambiguous-association rejection | **PASS** | Turn `4a824f52-9e0f-4078-b90d-89d39e12c3c7` fetched public Bratislava source shapes but promoted none of the ambiguous date/definition associations. Validation remained recovery and the terminal was a safe shortfall at 0/2 with one outcome/done. The minimized sanitized regression rejects the newly observed shape while valid labelled tables and renderable conflicts remain covered. |
| 6 | Missing credential, HTTP 429, timeout, and malformed Tavily response | **PASS** | Turns `5e9d3cb9-03f4-4222-9920-f9ec8a200d69`, `b6b8133d-3c5d-41e5-b81c-61c5acb9a7e1`, `2c5ba7ee-e4d7-4b18-8651-c4cd8ac5b8d5`, and `ddacc380-c06c-4082-8a74-9709f6d365a9` each recorded one Tavily attempt, zero Tavily retries, one challenged DuckDuckGo fallback, zero fetches, a safe shortfall, one outcome, and one done. Typed Tavily states were `failed(connectorunavailable)`, `failed(ratelimited)`, `timedout`, and `invalidresponse`. |
| 7 | Acceptance fault route boundary | **PASS** | With the acceptance flag present, the route returned 401 unauthenticated and 200 authenticated. After both Stage 8 flags were removed and the signed daemon restarted, the authenticated route returned 404. |
| 8 | Rollback with evidence flag absent | **PASS** | The signed rollback turn emitted zero typed evidence events, zero Tavily events, one nonempty token event, one done, and no error. The fault route remained 404. |
| 9 | Keychain-only credential lifecycle | **PASS** | The Tavily Keychain item was present without reading or printing its value. Posting `api_key:null` cleared daemon memory. Follow-up turn `a0df022e-d1e3-4cbc-8cfc-656714a64bce` used Wikipedia/DuckDuckGo and recorded zero Tavily activity, proving the cleared value was not reused. Static scans found no live key or private-key material. |
| 10 | Canonical survival under signed preferred-model unavailability | **PASS** | The registered 35B package was temporarily withheld outside the BaseRT model roots and BaseRT restarted. Turn `4c17b95b-df4e-4be2-9469-4d4d79dd3b26` had an eligible partial canonical bundle, recorded 35B `model_unavailable`, polish `unavailable`, deterministic rendering, nonempty canonical output, one outcome, and one done. The package and both registry entries were restored immediately and verified before cleanup. No production lifecycle code or policy changed. |
| 11 | Privacy-safe diagnostics and one terminal per turn | **PASS** | Sixteen structural evidence exports each contained exactly one outcome and zero forbidden diagnostic keys. SSE scans contained zero prohibited internal keys. Every qualifying signed evidence turn contained exactly one done. |
| 12 | UI results against established baseline | **PASS** | Swift passed 39/39 and focused evidence presentation passed 4/4. The exact-revision shell baseline passed inline focus, no-overflow promotion, and thinking/output-dot; it retained the same four documented failures for fullscreen-notch visibility, non-notch pill, legacy Mail shell, and output-scroll stability. |
| 13 | Final runtime cleanup | **PASS** | Both Stage 8 flags are absent. The signed app, daemon, and BaseRT processes are stopped; port 8082 has no listener; daemon and BaseRT LaunchAgents are unregistered. The 35B package and registry directory are present, and no model remains loaded because the runtime is stopped. |

## Red-to-green remediation record

- Mail: `transient_unavailable_body_retries_once_and_can_complete_the_reading_batch`
  failed before the bounded retry correction and passes afterward.
- Authority: discovery/fetched-owner binding regressions fail without fetched
  institutional identity and pass without static URL or domain-name trust.
  The fetched institutional-title/pronoun-biography regression failed before
  the correction and passes afterward.
- Entity binding: institutional-name, unrelated born-subject, and competing-name
  pronoun cases failed against their preceding revisions and now fail closed;
  the valid single-person official biography remains eligible.
- Canonical terminal: complete corroborated bundles with two independently
  grounded claims and two citations failed before shared completeness/outcome
  derivation and pass afterward without weakening claim validation.
- Bratislava: the minimized ambiguous flattened source shape failed before the
  safety correction and passes afterward. Multiple dates, publication context,
  unscoped figures, uncertain definitions, and competing definitions remain
  rejected; labelled table and conflict fixtures remain valid.

## Automated, privacy, release, and signing validation

- Complete Rust workspace and all targets: pass. `bagentd` passed 252 tests
  with three explicit live/acceptance-only ignores; all remaining crate and
  protocol suites passed.
- Complete Swift: 39/39; focused evidence presentation: 4/4.
- Focused Mail, web, validator, canonical renderer, events, diagnostics,
  provider-fault, and synthesis/lifecycle regressions: pass.
- `cargo fmt --all -- --check` and `git diff --check`: pass.
- `cargo clippy --workspace --all-targets`: pass with established unrelated
  warnings.
- Static credential/privacy scan: no live credential or private-key material;
  only deterministic test placeholders match the Tavily prefix.
- `cargo build --release --workspace`, release `make bundle`, and
  `codesign --verify --deep --strict`: pass. Bundle identifier
  `sk.bagent.app`; Apple Development signature; team `QUB47S3XTF`.
- Independent specification review: **PASS** on exact commit
  `8536f0115a50e2c011b39ce31ac79cc16b47d8fc`.
- Independent standards review: **PASS** on exact commit
  `8536f0115a50e2c011b39ce31ac79cc16b47d8fc`.

## Scope and protected state

No Mail permission was changed. No Mail write, approval decision, automation
run, static web candidate, production model lifecycle hook, or Stage 9 action
was introduced. Executed web URLs remained typed discoveries or validated
fetched outbound references.

User-owned untracked `.scratch/`, `CONTEXT.md`, and
`docs/KHOJ_BASERT_TAILSCALE_RESEARCH.md` were preserved unchanged and are not
included in either Stage 8 commit.

## Decision

Stage 8 remains **FAIL** because mandatory criteria 1–4 did not all pass in the
same immutable signed revision. Criteria 5–13 pass, including the previously
missing signed preferred-model-unavailability acceptance. Stage 9 remains
unauthorized.

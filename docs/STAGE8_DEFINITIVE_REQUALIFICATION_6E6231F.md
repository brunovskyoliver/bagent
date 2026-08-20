# Stage 8 definitive requalification against 6e6231f

Date: 2026-07-30

Revision under test: `6e6231f300351c5003f5f1aff9d27f3a62e2b047`

Overall verdict: **FAIL**

Stage 9 implementation: **NOT AUTHORIZED**

The first signed campaign on the exact commit appeared to pass. Mandatory final
standards/specification review then found reproducible provenance and conflict
correctness defects. Four narrow deterministic corrections were made under the
acceptance exception and the complete automated and signed-app campaigns were
repeated. The corrected campaign failed five mandatory live criteria, so the
earlier apparent PASS is superseded by this report.

Two source states are intentionally distinguished:

- exact target commit `6e6231f300351c5003f5f1aff9d27f3a62e2b047`;
- corrected uncommitted patch SHA-256
  `296eedefe554e9055a5379cc9e0e74303b85de8446e67de0c21f854ef4fe7b43`,
  built as signed app executable SHA-256
  `d8cb0e834d10e16ed51e8bebe8b099649bdceb22124cfd3c30d6859b276a7d19`
  and daemon executable SHA-256
  `6be0b9d0d254bbee7d9e7ad01e73830a278ff3e6dd27ef67428ee96142cd8514`.

## Exact-target campaign record

The exact-commit signed campaign produced the following structural record. A
live-looking success is not a PASS when final review proves its provenance or
correctness contract false.

| # | Exact-commit result | Structural record and final qualification |
| ---: | --- | --- |
| 1 | **PASS** | Turn `7cb4f9a8-9f4f-460e-84f7-f72169d9c602`: one list, three distinct reads, 3/3 Mail bodies from automation, exactly three Sender/Subject/Date/Summary groups, one outcome and one done. |
| 2 | **FAIL** | Turn `74b7d483-25ef-47bd-90f6-98022a13a4b2` appeared first-party verified with one search/fetch and a `prezident.sk` citation. Review proved the URL was statically injected and falsely labelled as DuckDuckGo discovery; the authority result therefore lacked valid typed-discovery provenance. |
| 3 | **PASS** | Turn `66591bf5-be17-4e4a-8d8e-3a6f54501804`: two independent acquired sources, two claim-adjacent citations, complete validation, one outcome/done; rejected polish preserved the canonical result. |
| 4 | **PASS** | Turn `ec3c5019-458e-4de3-864e-4de22c849935`: two acquired sources, one conflict, two separate cited bullets and distinct figures; exact terminal `Web verified · 2 sources · conflict`. |
| 5 | **PASS** | Turn `d30220b1-4db1-4095-ad0b-2b5e02b8b12b`: two sources acquired but ambiguous associations were not promoted; safe shortfall, one outcome/done. |
| 6 | **PASS** | Turns `d900a721-63dd-4805-ab20-499bfd86a357`, `6abc09b5-0990-45b0-9e8b-9029b86ff53b`, `c1e87174-3b07-4b65-a84b-5866a537bb00`, and `71f9afb6-03ff-45c5-b6c5-6817478085d8`: one Tavily attempt, zero Tavily retries, at most one DuckDuckGo fallback, zero fetches, safe shortfall and one outcome/done each. |
| 7 | **PASS** | Acceptance flag present: unauthenticated 401/authenticated 200; default runtime after removal: authenticated 404. |
| 8 | **PASS** | Flags absent: legacy turn one done, zero typed evidence, zero Tavily; fault route 404. |
| 9 | **PASS** | Keychain item present; exact-key scans found zero repository/artifact/app-data/log/plist/process copies. Clearing daemon memory yielded a follow-up with zero Tavily provider activity. |
| 10 | **PASS** | Mail polish was unavailable; corroborated/conflict/Bratislava polish was rejected; each retained nonempty canonical output. |
| 11 | **PASS** | Ten evidence turns each emitted one outcome and one done; forbidden diagnostic/SSE key counts were zero. |
| 12 | **PASS** | Swift 39/39 and focused presentation 4/4; results matched the established three-pass/four-known-failure UI baseline. |
| 13 | **PASS** | Model registry explicitly reduced from two to zero, then app/daemon/BaseRT/listeners/LaunchAgents/flags were clean. |

Default routing remained disabled. `BAGENT_EVIDENCE_ORCHESTRATOR=1` and
`BAGENT_STAGE8_ACCEPTANCE_PROVIDER_FAULTS=1` were present only during the
isolated acceptance campaign. No Stage 9 flag or implementation was enabled.

This report contains structural evidence and public domains only. It contains
no Mail content, private prompt, raw connector identifier, credential, fetched
passage, or model response.

## Mandatory criteria

| # | Criterion | Verdict | Exact corrected signed-app evidence |
| ---: | --- | --- | --- |
| 1 | Exact three-email Mail content flow | **FAIL** | Turn `6564e59a-a245-4b5b-aea5-4cf2fde2e509` executed one `mail.list` and three distinct `mail.read` activities, but acquired only 1 of 3. The answer contained one Sender, Subject, Date, and Summary; body origins were `mail_automation` and `unavailable`. Validation was `bundle_partial`; terminal was `Read 1 of 3 emails · partial`. One outcome and one done event. |
| 2 | Relevant authoritative first-party web answer | **FAIL** | Turn `65e29604-ad07-4575-ad1e-8ff2d25d67de` ran two bounded discovery rounds and five fetch activities but recorded zero eligible first-party fetches and acquired 0 of 1. Validation entered recovery; terminal was `Couldn't verify sources`. The official site appeared among fetched domains, but no fetched claim met the first-party authority contract, so it was not promoted. |
| 3 | Two independently grounded corroborated claims | **FAIL** | Turn `7bf8aeee-582f-4f72-828f-03ab7aade6d1` acquired 2 of 2 independent domains (`cepa.org`, `planetrulers.com`) with two citations and `bundle_complete`, but the canonical terminal remained `Couldn't verify sources`. Rejected polish did not repair or replace it. |
| 4 | Renderable conflict | **PASS** | Turn `03084ef6-1ae8-4cb9-9d46-8589f1fbf6ca` acquired 2 of 2, recorded `conflict_count=1`, rendered two separately cited bullets with two distinct figures, and ended exactly `Web verified · 2 sources · conflict`. Rejected polish left the canonical conflict intact. |
| 5 | Bratislava ambiguous-table safety | **FAIL** | Turn `7eba300e-fe8e-40ed-93d8-9ac280c6ef5a` acquired two sources and incorrectly ended `Web verified · 2 sources` instead of the required safe shortfall. Validation was `bundle_complete`; rejected polish preserved the unsafe terminal classification. |
| 6 | Missing credential, 429, timeout, malformed Tavily response | **PASS** | Turns `72c23540-ca56-486d-9c74-7759a1a65f2a`, `5a3b1f71-7f8e-4bda-81e7-caea76423d32`, `b4c64aa9-5841-4d53-9596-85045deda16f`, and `3e9bffd7-0504-400f-b4f5-d41b88101c16` each recorded exactly one Tavily attempt, zero Tavily retries, one DuckDuckGo fallback, zero fetches, one outcome, one done, and a safe shortfall. Typed Tavily statuses were `failed(connectorunavailable)`, `failed(ratelimited)`, `timedout`, and `invalidresponse`. |
| 7 | Acceptance fault route boundary | **PASS** | With the explicit fault flag, unauthenticated control returned 401 and authenticated control returned 200. After both flags were removed and the signed daemon restarted, the authenticated path returned 404. |
| 8 | Rollback with evidence flag absent | **PASS** | With `BAGENT_EVIDENCE_ORCHESTRATOR` absent, a signed legacy turn produced six visible characters, one done, zero typed evidence events, and zero Tavily events. The authenticated fault route was 404. Automated rollback coverage also passed. |
| 9 | Keychain-only credential lifecycle | **PASS** | The Keychain item remained the credential source; no credential was written to repository or report. Posting `api_key:null` cleared daemon memory. Follow-up turn `47cb8385-7210-4125-8bd1-cb18915d7427` emitted Wikipedia/DuckDuckGo provider activity and zero Tavily provider activity, proving the cleared value was not reused. |
| 10 | Canonical survival after rejected/unavailable 35B polish | **FAIL** | Mail, corroborated, conflict, and Bratislava turns retained nonempty canonical output after rejected polish, but the corrected signed campaign did not reproduce an unavailable 35B-polish path. Automated unavailable/timed-out/memory-ineligible/poisoned-process/restart-before-fallback coverage passed, but it is not signed-app confirmation. |
| 11 | Privacy-safe diagnostics and one terminal per turn | **PASS** | All ten corrected evidence turns emitted exactly one Evidence Outcome and one done event. Structural exports contained 8–60 diagnostic events per turn and zero forbidden diagnostic keys; SSE structural scans found zero forbidden internal keys. |
| 12 | UI results against established baseline | **PASS** | No UI implementation changed. Complete Swift remained 39/39 and focused evidence presentation 4/4. The established baseline comparison is unchanged: inline focus, no-overflow promotion, and thinking/output-dot checks pass; the same four documented pre-existing failures remain for fullscreen-notch visibility, non-notch pill, legacy Mail shell, and output-scroll stability. |
| 13 | Final runtime cleanup | **PASS** | The 35B and 4B models were explicitly unloaded; authenticated `/v1/models` moved from two entries to zero. Final inspection found zero app/daemon/BaseRT executables, zero port-8082 listeners, daemon and BaseRT LaunchAgents unregistered, and both acceptance flags absent. |

## Provider-failure structure

| Fault | Tavily attempts | Tavily retries | DuckDuckGo fallbacks | Unsupported fetches | Terminal |
| --- | ---: | ---: | ---: | ---: | --- |
| Missing credential | 1 | 0 | 1 | 0 | `Couldn't verify sources` |
| HTTP 429 | 1 | 0 | 1 | 0 | `Couldn't verify sources` |
| Timeout | 1 | 0 | 1 | 0 | `Couldn't verify sources` |
| Malformed response | 1 | 0 | 1 | 0 | `Couldn't verify sources` |

## Deterministic corrections required by final review

The review reproduced defects in the exact target commit before any correction:

- authoritative mode fabricated static `prezident.sk` candidates and labelled
  them as DuckDuckGo discoveries;
- contradictory office-holder prose was not preserved as a conflict;
- explicit office-holder negation was not preserved as a conflict;
- equivalent numeric formats such as `8,848.86` and `8848.86` could be rendered
  as different figures.

Red tests were added first. The corrections remove undiscovered static
candidates, detect positive and explicit-negation office-holder conflicts, and
share canonical numeric normalization between validation and rendering. These
changes are limited to three daemon source files. They remain uncommitted
because the mandatory live requalification failed.

## Automated, build, and signing verification

- Focused Rust: web 43/43, orchestrator 30/30, diagnostics 4/4, events 7/7,
  deterministic web 3/3, deterministic conflict rendering 1/1, fault control
  1/1, synthesis/lifecycle 43 passed with two live tests ignored, and BaseRT
  protocol 13/13.
- Complete Rust workspace: pass; `bagentd` 235 passed with three explicit
  live-only ignores; every remaining crate and doc-test suite passed.
- Complete Swift suite: 39/39; focused evidence presentation: 4/4.
- `cargo fmt --all -- --check`, `git diff --check`: pass.
- `cargo clippy --workspace --all-targets`: pass with established warnings.
- Static privacy/credential scan: no live credential or private-key material;
  the only matching `tvly-` source string is a deterministic placeholder.
- `cargo build --release --workspace` and release `make bundle`: pass.
- `codesign --verify --deep --strict`: pass for identifier `sk.bagent.app`,
  Apple Development authority, team `QUB47S3XTF`.

## Scope and protected state

The pre-correction campaign confirmed the protected-state snapshot was
unchanged: `rules.yaml` SHA-256
`6b8db65652bb2e5349349d17ce3942f34e714fbd0a0eb0d70b1779400afb0bc0`;
approval rows 0; pending approvals 20; automations 1; `chat_turns` 0; approval
and automation surface hashes identical. The corrected campaign performed the
same read-only flows and acceptance-only provider fault controls. No approval
decision, automation run, Mail write, permission change, or Stage 9 action
occurred.

User-owned untracked `.scratch/`, `CONTEXT.md`, and
`docs/KHOJ_BASERT_TAILSCALE_RESEARCH.md` were preserved. Private temporary live
artifacts are removed after review; they are not committed.

## Decision

The exact target commit fails criterion 2 because its apparent first-party
success used fabricated discovery provenance. Mandatory criteria 1, 2, 3, 5,
and 10 fail or remain unverified in the pinned corrected signed release app.
Stage 8 is **FAIL** and **Stage 9 implementation remains unauthorized**. The
narrow blocker is the live evidence pipeline's inability to reliably complete
the exact Mail, authoritative, corroborated-terminal, and ambiguous-table
safety contracts without weakening provenance or validation, plus the missing
signed unavailable-polish confirmation.

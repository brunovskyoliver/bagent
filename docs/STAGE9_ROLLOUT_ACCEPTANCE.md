# Stage 9 rollout and rollback acceptance

Date: 2026-07-30

Accepted Stage 8 candidate: `96756b27c1d5798887e3baa379d5b0ab449bbc09`

Stage 9 implementation revision: `945b2324e91caa1a722d6c309d854c596883388b`

Post-validation documentation-only formatting revision: `7441ea4`

## Verdict

Stage 9 passes under ADR-0002. The typed evidence orchestrator is the ordinary
production default for the already supported deterministic evidence intents.
The existing `BAGENT_EVIDENCE_ORCHESTRATOR=0` control restores the prior
agentic loop after daemon restart without migration, protected-state mutation,
credential mutation, or fixture exposure.

The implementation commits are the contiguous range `3e5d945..7441ea4` on top
of the accepted Stage 8 record `ef3c7b0`. The rollout report is committed
separately from that implementation range.

## Routing matrix

| Request class | Flag absent | Flag `1` | Flag `0` | Invalid value |
|---|---|---|---|---|
| Latest Mail headers | Typed | Typed | Legacy | Typed + normalized warning |
| Latest Mail content | Typed | Typed | Legacy | Typed + normalized warning |
| One direct web page | Typed | Typed | Legacy | Typed + normalized warning |
| Simple authoritative web fact | Typed | Typed | Legacy | Typed + normalized warning |
| Corroborated web fact | Typed | Typed | Legacy | Typed + normalized warning |
| Supported quoted-evidence wrapper | Typed | Typed | Legacy | Typed + normalized warning |
| Targeted or ambiguous Mail | Legacy | Legacy | Legacy | Legacy |
| Mixed Mail/web | Legacy | Legacy | Legacy | Legacy |
| Unsupported, ambiguous, unrelated, or ordinary agentic use | Legacy | Legacy | Legacy | Legacy |

Chat and automation use the same routing decision. Typed turns receive no
legacy Mail guidance, prefetch, or tool registry, so the model cannot decide
whether mandatory evidence operations run and duplicate legacy work cannot
occur.

## Signed default acceptance

The ordinary feature-absent bundle and the explicit acceptance-feature bundle
were signed with identifier `sk.bagent.app` and Team ID `QUB47S3XTF`; strict
deep signature verification passed.

The final acceptance-feature bundle at the implementation revision ran the
ADR-0002 deterministic signed suite twice: 21 cases per campaign, campaigns
identical. It covered the exact three-message Mail contract, header-only Mail,
partial/unavailable/denied/empty Mail, authoritative/corroborated/conflicting
web evidence, verification shortfall, Tavily/DDG provider failures, accepted,
rejected and unavailable polish, one Evidence Outcome, one `done`, and no
duplicate Mail/provider work.

All 21 canonical token hashes, citation-set hashes, and byte counts matched the
frozen Stage 8 values. The complete privacy-safe acceptance artifact SHA-256 is
`1422d9e89c694ec317672de69bf5046c9581320c803ddca002246a40b5d6fcbe`,
identical to the prior definitive Stage 8 artifact.

The installed ordinary bundle returned HTTP `404` for the acceptance fixture
route, proving that fixture controls are absent from the ordinary build.

## Signed rollback acceptance

With `BAGENT_EVIDENCE_ORCHESTRATOR=0`, the signed ordinary bundle restarted
with a changed daemon PID and executed four legacy Mail tool calls. It emitted
zero typed evidence events and zero Tavily provider activity. The local BaseRT
legacy completion ended in exactly one normalized privacy-safe
`model_unavailable_basert` terminal after the observed local model failure; no
answer bytes were emitted. This is a safe unavailable observational result
under ADR-0002, not an unsafe or falsely grounded answer.

Rollback activation and the legacy turn left all protected database tables,
rules, attachments, daemon token, and Tavily Keychain metadata unchanged. The
test reused an existing opaque session and verified that restoration removed
the rollback flag. Empty session persistence on app startup was retired so a
restart itself performs no stored-session write.

## Observational smoke

The ordinary default-enabled bundle was exercised without retaining prompt,
Mail, URL, answer, rowid, or credential content:

- latest Mail headers: `verified`, 3 of 3, one outcome and one `done`;
- latest Mail content: safe `partial`, 2 of 3, one outcome and one `done`;
- authoritative web: safe `verification_shortfall`, 0 of 1, one outcome and one `done`;
- corroborated web: safe `verification_shortfall`, 0 of 2, one outcome and one `done`.

No unsafe observational answer occurred. The Tavily item remained in macOS
Keychain and no credential value was read or persisted by the acceptance
artifacts.

## Validation

- `cargo test --workspace`: PASS (live tests requiring external services remain explicitly ignored).
- `cargo test -p bagentd --features stage8-acceptance`: PASS, 256 passed and 3 ignored in the final full run.
- Focused routing, evidence, provider, rendering, lifecycle, and normalized-error regressions: PASS.
- `swift test --package-path apps/macos`: PASS, 41 tests.
- Deterministic signed E2E twice: PASS, 21 cases per campaign and identical canonical hashes.
- Signed ordinary fixture boundary: PASS (`404`).
- Signed rollback routing/state/Keychain acceptance: PASS with safe unavailable terminal.
- Observational Mail/Tavily smoke: PASS under ADR-0002 with the safe partial/shortfall states above.
- `cargo fmt --all -- --check` and `git diff --check`: PASS.
- `cargo clippy --workspace --all-targets`: PASS with existing warnings.
- Strict `cargo clippy --workspace --all-targets -- -D warnings`: does not pass because of pre-existing unrelated workspace lint debt in rules, memory, Codex, Mail, filesystem, agent, and WhatsApp code; Stage 9's own reported needless-borrow findings were fixed.
- Credential literal scan: PASS, zero key/private-key patterns in added lines. `gitleaks` was unavailable locally.
- Privacy scan: PASS; the only text match for raw Mail rowids is this documentation's negative guarantee, not data or an exposure path.
- Independent specification review: PASS at the implementation revision.
- Independent standards/privacy review: PASS at the implementation revision.

## Emergency rollback and restoration

Rollback, with no secrets embedded:

```sh
launchctl setenv BAGENT_EVIDENCE_ORCHESTRATOR 0
osascript -e 'tell application "bagent" to quit'
open -a bagent
```

Restore the production default:

```sh
launchctl unsetenv BAGENT_EVIDENCE_ORCHESTRATOR
osascript -e 'tell application "bagent" to quit'
open -a bagent
```

The change affects routing only, requires no reinstall, and becomes effective
after daemon restart.

## Final runtime requirement

The final handoff leaves `/Applications/bagent.app` as an ordinary signed
feature-absent bundle. The evidence, fixture, fault, and structured-synthesis
environment flags are absent; the app, daemon, BaseRT service, model loads, and
acceptance harness are stopped. The Tavily Keychain item and unrelated
untracked workspace files are preserved. Nothing is pushed.

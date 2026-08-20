# Stage 8 final integration addendum

Date: 2026-07-29

Baseline: `68fdd2b57ff1dca1fc2726221e52c4629140c644`

Stage 9 default enablement: **not authorized**. Evidence routing remains disabled by default.

## Acquisition diagnosis and fixes

The canonical acceptance report describes the failed authoritative, corroborated, and conflict cases, but does not retain their exact request strings. Therefore exact byte-for-byte query reproduction is not possible from that artifact. The described authoritative and corroborated shapes were rerun against the signed bundle; the public conflict shape was also attempted through the same two-independent-source contract.

Privacy-safe acquisition diagnostics now retain provider status, candidate count, normalized registrable source identity, candidate rank, authority class, relevance score, extraction result, normalized rejection reason, and search/fetch budget consumption. They retain no query, snippet, passage, URL, Mail metadata, or connector identifier.

Observed root causes are separate:

- DuckDuckGo Lite returned a typed provider challenge on both permitted searches. Wikipedia succeeded.
- The first search lacked an eligible first-party candidate and the deterministic diversified second query still returned one publisher identity.
- URL-only deduplication allowed fetch capacity to be spent on pages that collapsed to `wikipedia.org`.
- ranked title metadata could over-promote an apparent official page; first-party classification now requires an institutional suffix or an organization named by the request plus explicit official-site metadata, never rank alone.
- fetched reference pages previously exposed only same-host links, preventing validated outbound discovery.
- several fetched pages extracted successfully but had no claim-relevant passage.

The implementation now diversifies registrable source identities before repeated publishers, preserves corroborated capacity for independent identities, runs the deterministic second query when authority/independence is absent, admits query-relevant outbound references, and validates every discovered URL through the unchanged fetch/DNS/redirect/SSRF/final-URL path. Public provider challenges remain typed and visible.

## Deterministic conflict coverage

`evidence::orchestrator::tests::corroborated_web_enforces_independence_retry_budget_duplicates_and_concurrency` is deterministic in-memory conflict coverage. It uses two independently typed publishers, preserves both differing numeric claims, emits one typed conflict, and performs no public network or model call. The canonical conflict renderer test verifies both adjacent citations and does not choose a winner. It is not a fixture served over isolated local HTTP infrastructure, so that narrower acceptance item remains open.

## Mail provenance

Hydration now records `body_origin = local_emlx | mail_automation | unavailable` at the connector completion point, carries it in typed Mail body evidence and completed Logical Activity events, and permits only that enum plus timing/status in diagnostic export. Local, Automation, unavailable, event, typed-result, and privacy-export tests pass.

Signed correlated final-code turn `09b82a29-9d0f-4c22-819e-1e4fe12e1d0a` read one naturally uncached message. Its completed `mail.read` activity reported `mail_automation`, it became readable, and the terminal outcome was `1 of 1` verified. An earlier ten-message turn on the same provenance implementation also reported ten `mail_automation` reads and verified 10 of 10. No TCC or Mail permission was changed. Only structural event fields were retained.

## Live requalification

| Case | Result |
| --- | --- |
| Authoritative public case | Safe shortfall. Both searches saw Wikipedia results and a typed DuckDuckGo challenge; no eligible first-party passage was fetched. |
| Corroborated public case | Safe 1-of-2 partial result from `wikipedia.org`; the second search could not supply an independent publisher. |
| Public conflict case | Safe shortfall because two independent public publishers could not be acquired; no independence or grounding rule was weakened. |
| Naturally uncached Mail | Pass: correlated signed turn above reported `mail_automation` and verified 10 of 10 readable bodies. |

The public positive web acceptance cases therefore remain blocked by the live provider challenge. Safe shortfalls are correct but do not authorize Stage 9.

## Follow-up artifacts

- `docs/followups/STAGE8_FULLSCREEN_NOTCH_VISIBILITY.md`
- `docs/followups/STAGE8_NON_NOTCH_INLINE_PILL.md`
- `docs/followups/STAGE8_LEGACY_MAIL_OUTPUT_REGRESSION.md`
- `docs/followups/STAGE8_OUTPUT_SCROLL_STABILITY.md`

Each records a baseline-identical failure outside evidence-default-enablement scope. No UI fix is included here.

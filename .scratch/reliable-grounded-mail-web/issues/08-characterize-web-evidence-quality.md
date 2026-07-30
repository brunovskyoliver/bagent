# Characterize web search and fetch evidence quality

Type: research
Status: resolved
Blocked by: 01

## Question

Once `web_search` and `web_fetch` are actually invoked, what structured evidence and failure signals do the current DuckDuckGo, Wikipedia, and page-fetch implementations provide across direct domains, redirects, missing pages, dynamic sites, conflicting results, weak sources, and same-site subpages? Determine which source-selection and sufficiency decisions can be deterministic, which require model judgment, and what connector result changes the evidence contract may require.

## Answer

The implementation analysis, controlled probes, primary sources, and required result shapes are recorded in [Web search and fetch evidence quality](../assets/08-web-evidence-quality.md).

Current search/fetch output is useful text but not a trustworthy evidence state:

- Wikipedia and DuckDuckGo are flattened into untyped lines; provider failure, challenge, invalid response, and genuine zero results can collapse together.
- DuckDuckGo varied between normal HTTP 200 results and an HTTP 202 bot challenge during controlled probes. The challenge is silently interpreted as no DDG results and can be masked by Wikipedia candidates.
- Fetch failures, unsupported content, empty extraction, redirects, truncation, and readable evidence are prose strings rather than typed outcomes. Audit can record `ok: true` even when the result says `Fetch failed`.
- Redirected pages are cited using the requested URL rather than the final URL.
- Dynamic pages, boilerplate-heavy HTML, exact-host link extraction, and the 6,000-character cap provide no extraction-quality or completeness signal.
- Literal-host SSRF checks are useful but explicitly do not defend DNS rebinding or hostnames resolving to private addresses.

The connector/orchestrator must deterministically own provider status, candidate identity/rank, URL validation and deduplication, fetch/final URL/status/type/size/truncation/extraction status, redirect and SSRF safety, budgets, evidence counts, and citation eligibility. The model may rank semantic relevance, choose promising same-site subpages, assess claim support/conflicts/authority, and decide whether more exploration is useful inside fixed budgets; every selection and citation must be validated against typed candidate/evidence IDs.

Search and fetch must return typed internal results with per-provider statuses, stable candidate IDs, requested and final URLs, redirect chain, HTTP/content metadata, byte/character counts, readable/truncated state, links, and normalized failure codes. Only a successful fetch with useful readable content becomes Fetched Evidence or a Grounded Citation.

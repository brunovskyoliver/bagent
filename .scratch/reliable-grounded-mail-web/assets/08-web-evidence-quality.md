# Web search and fetch evidence quality

Research date: 2026-07-28 (Europe/Bratislava)

## Conclusion

The current tools return useful text, but not a trustworthy evidence state. `web_search`
flattens two providers into untyped lines and suppresses most Wikipedia failures.
`web_fetch` flattens success, redirects, truncation, and extraction quality into prose.
The orchestrator therefore cannot deterministically distinguish a verified page from a
search snippet, a partial extraction, a provider outage, or an anti-bot response.

The most consequential live finding is provider variability: DuckDuckGo Lite returned
an HTTP 202 bot challenge for one controlled query, while a separate `Alza.sk` probe
returned HTTP 200 with parseable result anchors. The implementation neither checks the
response status nor recognizes challenges; the challenged response produced zero DDG
results without an error. If Wikipedia returns anything, the combined search appears
successful even though general-web search failed.

The evidence contract should be enforced over typed connector results, not inferred
from strings by the model.

## What the implementation provides today

### `web_search`

- Calls Wikipedia REST search first with `limit=2`, in only `en` or `sk`. It extracts
  title, key, Wikidata-backed description, and an HTML excerpt, then constructs a
  Wikipedia article URL. The official REST schema also provides page id,
  `matched_title`, and thumbnail, but bagent drops them. The endpoint explicitly
  distinguishes `200 + pages: []`, `400`, and `500`; bagent ignores HTTP status and
  silently discards transport, JSON, and schema failures.
- Calls undocumented DuckDuckGo Lite HTML with a maximum of six parsed results.
  The brittle parser requires an exact double-quoted anchor prefix, extracts the next
  result snippet, and decodes `uddg` redirect wrappers. It does not return rank,
  provider status, result type, publication date, or source-quality metadata.
- Concatenates both providers as `title | url | snippet`. There is no provider field,
  stable result id, duplicate removal, canonical URL normalization, or separation of
  provider failure from zero results.
- Returns “No web results” only when the combined line list is empty. Consequently:
  two unavailable providers, a DDG challenge, malformed provider HTML, and a genuine
  zero-result search can all collapse to the same outcome.

Wikipedia’s official reference confirms that search results are relevance candidates,
not fetched evidence: the excerpt is only “a few lines” and may end mid-sentence.
DuckDuckGo says its traditional links are largely sourced from Bing and disclaims the
accuracy or completeness of third-party content. Search snippets therefore must remain
discovery material, never verified claim evidence.

### `web_fetch`

- Accepts only textual `http://` or `https://` inputs, rejects literal localhost,
  `.local`, loopback, private/link-local/unspecified IPv4, loopback/unspecified/ULA
  IPv6, and repeats the literal-host check on redirects.
- Uses one 10-second whole-request timeout and follows at most six redirects (the
  policy blocks after `previous().len() > 5`). It permits cross-domain redirects.
- Accepts an empty content type or any value containing `text/`, `json`, or `xml`;
  rejects other content types. It reads up to approximately 2 MB and emits at most
  6,000 characters.
- Uses a hand-written tag stripper, removes `script`, `style`, and `noscript` bodies,
  decodes only a small entity set, and does no readability/content-area selection.
  It does not execute JavaScript. Dynamic/JS-only pages therefore typically yield
  shell/navigation text or “No readable text,” even when a browser would show content.
- Extracts up to 30 same-host links from literal lower-case `<a ` elements. It handles
  relative URLs, removes fragments, removes duplicate absolute URLs, ignores external,
  `mailto:`, `javascript:`, fragment-only, and empty-label links, and truncates labels
  to 60 characters. Exact host equality means `www.example.com` and `example.com` are
  considered different sites.
- A non-2xx result yields an explicit HTTP failure, and network/body/content-type
  failures have recognizable prose. A 2xx response with empty readable text still
  starts with `Source:` and can be registered as a citation.
- Redirect identity is wrong: extraction uses the final URL for resolving links, but
  the returned `Source:` line contains the requested URL. The transcript source parser
  then registers that original URL/domain, not the final URL/domain.
- The byte cap can overshoot by one response chunk. Truncation is based on the stripped
  text length and is exposed only as a trailing `[truncated]` string; the orchestrator
  receives neither byte counts nor a completeness boolean.

The source registry only promotes `Source:` lines from `web_fetch`, which is directionally
correct. However it treats any such line—including “No readable text found”—as a
successfully opened, clickable source. Connector audit records `ok: true` whenever the
tool function returned, even if its text says `Fetch failed`.

## Controlled probes and scenario assessment

| Scenario | Current signal | Reliability / flaw |
|---|---|---|
| Wikipedia direct query | HTTP 200 JSON with page fields; bagent emits two flattened lines | Stable structured discovery, but not fetched evidence |
| Wikipedia missing query | HTTP 200 with `pages: []` | Officially distinguishable, but combined output cannot attribute the empty provider |
| DuckDuckGo general query | One live query returned HTTP 202 anti-bot challenge; another returned normal HTTP 200 results | Challenges silently parse as zero DDG results; provider degradation masquerades as a weak/encyclopedic-only search |
| Direct public page | On 2xx text-like content, returns capped stripped text and same-host links | No extraction-quality score, title, date, canonical URL, or provenance |
| Redirect | Follows public redirects and blocks literal private redirect targets | Returned/cited source remains the requested URL rather than final URL |
| Missing page | `Fetch failed: HTTP 4xx` | Useful signal, but untyped and audit still records success |
| Dynamic/JS site | Scripts and noscript content are discarded; no JS execution | Shell text/empty extraction can be mistaken for page evidence |
| Binary/download | Most non-text content types rejected | Correctly prevents arbitrary binary evidence; empty/mislabelled content type remains accepted |
| Same-site subpage | Up to 30 exact-host links returned | Good discovery seam, but selecting the promising link is prompt-only model judgment |
| Conflicting results | Independent flat result lines | No dates, provider/source class, claim association, or conflict flag |
| Weak source | Any public result/fetched textual page is accepted | No deterministic source tier or corroboration state |

Probes used only public, non-private requests:

- `GET https://en.wikipedia.org/w/rest.php/v1/search/page?q=OpenAI&limit=2`
  returned documented fields and HTTP 200.
- A unique nonexistent Wikipedia query returned HTTP 200 and `{ "pages": [] }`.
- `GET https://lite.duckduckgo.com/lite/?q=OpenAI` returned HTTP 202 and an image
  challenge (“bots use DuckDuckGo too”), with no parseable result anchors. A separate
  `Alza.sk` query returned HTTP 200, ten result links/snippets, and twelve exact anchor
  prefixes, confirming that the provider behavior varies by request rather than being
  uniformly unavailable.
- A unique nonexistent `example.com` path returned HTTP 404.

The three focused local parser/security tests pass:
`parse_ddg_lite_extracts_results_and_decodes_redirect_urls`,
`extract_links_resolves_same_site_and_skips_junk`, and
`private_hosts_are_rejected`. They test fixtures and literal hosts, not live provider
shapes, dynamic extraction, redirects, typed failures, or DNS rebinding.

## Security boundary

The literal-host checks block common direct SSRF inputs and repeat at each redirect.
They do **not** resolve hostnames and validate every resolved address before connecting,
pin the validated address for the request, or defend DNS rebinding; the source comment
acknowledges this. Other gaps include IPv4-compatible/mapped IPv6 and public DNS names
that resolve to private/link-local destinations. This is not a sufficient SSRF boundary
for an agent-controlled URL fetcher.

Required deterministic behavior:

1. Parse with `Url`, allow only HTTP(S), reject credentials, normalize host/port.
2. Resolve DNS, reject every non-global address (including metadata ranges), connect
   only to a validated address, and re-run this validation for every redirect.
3. Cap redirects, bytes, time, decompressed size, and supported media types with typed
   failure codes.
4. Return requested URL, final URL, redirect chain, status, media type, byte/character
   counts, truncation, and extraction outcome separately.

## Deterministic decisions versus model judgment

### Orchestrator/connector must decide

- Whether each provider succeeded, returned zero candidates, was blocked/challenged,
  timed out, or produced an invalid response.
- Search result identity, provider, rank, URL validity, deduplication, and domain.
- Whether a fetch succeeded, which final URL was fetched, whether the body was
  supported/readable/truncated, and whether it contributed non-empty evidence.
- SSRF and redirect safety, operation budgets, retry/deduplication, and the agreed
  requirement that snippets do not satisfy factual evidence.
- Whether at least one fetched first-party/primary source exists for a simple fact,
  and whether two distinct fetched domains exist where the contract requires
  corroboration. “Distinct domain” must not be confused with “independent source.”
- Citation eligibility: only a successful fetch with useful extracted content; cite the
  final canonical URL.

### Model may judge, with validation

- Which candidates are relevant to the user’s question.
- Which same-site subpage is promising.
- Whether extracted passages actually support a proposed claim.
- Whether two sources materially conflict and how to explain the disagreement.
- Source authority and independence where deterministic metadata cannot establish it.
- Whether more searching/fetching is useful inside the fixed two-search/five-fetch
  budget.

The orchestrator must validate the model’s selection against candidate ids and its
citations against fetched evidence ids. The model must not declare provider success,
fetch completeness, source independence, or contract satisfaction.

## Required connector result changes

Return typed JSON internally (UI/model rendering may remain textual):

```json
{
  "search": {
    "query": "...",
    "providers": [
      {"name": "wikipedia", "status": "ok", "result_count": 2},
      {"name": "duckduckgo", "status": "blocked_challenge", "result_count": 0}
    ],
    "results": [
      {
        "id": "candidate-...",
        "provider": "wikipedia",
        "rank": 1,
        "title": "...",
        "url": "...",
        "domain": "...",
        "snippet": "...",
        "matched_redirect_title": null
      }
    ]
  }
}
```

```json
{
  "fetch": {
    "status": "ok",
    "requested_url": "...",
    "final_url": "...",
    "redirect_chain": [],
    "http_status": 200,
    "content_type": "text/html",
    "bytes_read": 1234,
    "text_chars": 900,
    "truncated": false,
    "extraction": "readable",
    "title": null,
    "text": "...",
    "links": [],
    "failure_code": null
  }
}
```

At minimum, provider failures must not collapse into “no results”; a DDG challenge must
be explicit; successful fetch must require useful readable evidence; redirected pages
must cite their final URL; and all failure/audit/activity states must derive from typed
status rather than substring matching.

## Primary sources and code evidence

- [MediaWiki REST API reference: Search pages](https://www.mediawiki.org/wiki/API:REST_API/Reference#Search_pages)
  documents the result schema, excerpt limitation, empty-result response, parameters,
  and HTTP failure responses.
- [DuckDuckGo: Where search results come from](https://duckduckgo.com/duckduckgo-help-pages/results/sources)
  explains that traditional links are largely sourced from Bing and that results
  combine multiple third-party sources.
- [DuckDuckGo Terms](https://duckduckgo.com/terms) says third-party search content is
  not guaranteed accurate or complete and the service is available without warranties.
- Local implementation: `crates/daemon/src/main.rs:4274-4627`.
- Tool registration, policy gate, audit, and source promotion:
  `crates/daemon/src/agent_exec.rs:374-404`,
  `crates/daemon/src/agent_exec.rs:1435-1538`, and
  `crates/daemon/src/agent_exec.rs:1618-1649`.

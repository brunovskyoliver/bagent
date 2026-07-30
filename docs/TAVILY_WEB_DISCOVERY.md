# Free Tavily web discovery

Bagent can use Tavily's free Search API as candidate discovery for the default typed evidence
route. The legacy agentic web tool remains on Wikipedia plus DuckDuckGo so it cannot consume an
unbounded number of free credits. Tavily snippets remain discovery-only: every selected result is
still fetched by bagent and must pass its URL, DNS, redirect, peer-IP, SSRF, extraction, relevance,
authority, and independent-source validation before it can support an answer.

## Configuration

Create a free Tavily API key, then save it in the macOS Keychain without printing it or writing it
into the repository:

```zsh
read -rs "TAVILY_API_KEY?Tavily API key: "; echo
security add-generic-password -U -s sk.bagent.app -a bagent.tavily.apikey -w "$TAVILY_API_KEY"
unset TAVILY_API_KEY
```

Quit and reopen bagent after setting the value so its LaunchAgent restarts with the new environment.
The signed app reads the key and sends it over the authenticated loopback daemon API. The daemon
keeps it only in memory for that process lifetime. To remove the credential:

```zsh
security delete-generic-password -s sk.bagent.app -a bagent.tavily.apikey
```

Quit and reopen bagent after deletion; startup synchronizes the absent Keychain value and clears
any credential retained by the newly started daemon.

The daemon never logs or persists the key. It is used only to authenticate the fixed
`https://api.tavily.com/search` endpoint. Do not add the key to a plist, launch environment, shell
profile, `.env` file, or source-controlled configuration.

## Free-tier behavior

- Tavily uses `basic` search, with at most six discovery candidates and no generated answer, raw
  page content, or images.
- Each evidence turn permits at most two search operations. Each operation makes no more than one
  Tavily request, so a turn can consume at most two Tavily credits.
- Tavily requests are never retried. This avoids consuming a second credit after an indeterminate
  timeout and preserves the second provider slot for DuckDuckGo.
- HTTP 429/quota exhaustion, timeouts, invalid responses, and normalized provider failures remain
  typed provider outcomes. They fall back once to DuckDuckGo or produce a safe verification shortfall.
- If the key is absent, blank, or removed, routing automatically retains the previous Wikipedia and
  DuckDuckGo provider set.
- Bagent cannot enable Tavily pay-as-you-go. Keep the Tavily account on its free Researcher plan and
  do not add a payment method if paid overage is unwanted.

The typed evidence route is enabled by default. Configuring Tavily does not change routing.
`BAGENT_EVIDENCE_ORCHESTRATOR=0` restores the legacy agentic route after daemon restart; that
legacy web path remains Keychain-free and does not receive the Tavily credential.

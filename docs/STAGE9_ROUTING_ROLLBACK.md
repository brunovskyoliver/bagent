# Stage 9 evidence routing and rollback

Stage 9 enables the accepted typed evidence route by default without changing
the classifier or evidence contracts.

## Routing matrix

| Request | Flag absent | Flag `1` | Flag `0` |
|---|---|---|---|
| Latest Mail headers | Typed | Typed | Legacy |
| Latest Mail content | Typed | Typed | Legacy |
| One direct web page | Typed | Typed | Legacy |
| Simple authoritative web fact | Typed | Typed | Legacy |
| Corroborated web fact | Typed | Typed | Legacy |
| Supported quoted-evidence wrapper around an intent above | Typed | Typed | Legacy |
| Targeted or ambiguous Mail | Legacy | Legacy | Legacy |
| Mixed Mail/web or multiple ambiguous pages | Legacy | Legacy | Legacy |
| Unsupported, unrelated, or ordinary agentic request | Legacy | Legacy | Legacy |

An invalid value follows the absent production default and produces one
normalized configuration warning. The supplied value is not logged.

## Emergency rollback

Run exactly these commands in the signed-in user's macOS session:

```zsh
launchctl setenv BAGENT_EVIDENCE_ORCHESTRATOR 0
osascript -e 'tell application "bagent" to quit'
open -a bagent
```

The app passes `0` into the regenerated daemon LaunchAgent. The restart makes
rollback effective immediately for subsequent turns. This changes routing
only: it performs no migration and does not modify Mail, rules, approvals,
automations, credentials, or stored user data. It requires no app reinstall.

Release acceptance verifies that boundary against an ordinary signed bundle:

```sh
python3 scripts/stage9-rollback-acceptance.py \
  --app-bundle /Applications/bagent.app \
  --output /tmp/stage9-rollback.json
```

The report contains only structural event counts and hashes of protected local
tables. It never reads or emits Mail content, the daemon token, or the Tavily
secret. The script restores the absent-flag default after its rollback turn.

## Restore the production default

```zsh
launchctl unsetenv BAGENT_EVIDENCE_ORCHESTRATOR
osascript -e 'tell application "bagent" to quit'
open -a bagent
```

Absence and `1` are equivalent. Prefer absence for ordinary operation. The
Stage 8 fixture boundary is separate: ordinary builds do not compile the
fixture implementation, and the route remains 404 even if a fixture variable
is supplied. Never put a Tavily key in these commands or any environment
variable; it remains in the macOS Keychain.

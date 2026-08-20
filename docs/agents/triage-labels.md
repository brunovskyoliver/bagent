# Triage labels

The engineering skills use five canonical triage roles. Local Markdown tickets
record the corresponding value in their `Triage:` field.

| Canonical role | Local value | Meaning |
|---|---|---|
| `needs-triage` | `needs-triage` | A maintainer needs to evaluate the ticket. |
| `needs-info` | `needs-info` | The ticket is waiting on its reporter. |
| `ready-for-agent` | `ready-for-agent` | The ticket is fully specified and AFK-ready. |
| `ready-for-human` | `ready-for-human` | Human participation or implementation is required. |
| `wontfix` | `wontfix` | The ticket will not be actioned. |

When a skill refers to a canonical role, use the matching local value exactly.

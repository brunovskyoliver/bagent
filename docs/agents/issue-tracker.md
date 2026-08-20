# Issue tracker: Local Markdown

Issues and planning maps for this repository live as Markdown files under
`.scratch/`. External pull requests are not a triage request surface.

## Conventions

- One effort per directory: `.scratch/<effort-slug>/`.
- A Wayfinder map is `.scratch/<effort-slug>/map.md`.
- Child tickets are
  `.scratch/<effort-slug>/issues/<NN>-<slug>.md`, numbered from `01`.
- Assets created while resolving a ticket live under
  `.scratch/<effort-slug>/assets/` and are linked from the ticket.
- Triage state is recorded as a `Status:` line near the top of a ticket.
- Comments and conversation history append under `## Comments`.

## When a skill says "publish to the issue tracker"

Create or update the appropriate file under `.scratch/<effort-slug>/`,
preserving unrelated efforts and user-owned files.

## When a skill says "fetch the relevant ticket"

Read the referenced file. The user may identify it by path, ticket number
within the effort, or ticket title.

## Wayfinding operations

- **Map**: `.scratch/<effort>/map.md`.
- **Child ticket**:
  `.scratch/<effort>/issues/NN-<slug>.md`.
- **Ticket type**: a `Type:` line containing `research`, `prototype`,
  `grilling`, or `task`.
- **Open ticket**: `Status: open`.
- **Claim**: set `Status: claimed` and record `Assignee:` before doing any
  ticket work.
- **Blocking**: a `Blocked by: NN, NN` line. A ticket is unblocked only when
  every listed ticket has `Status: resolved`.
- **Frontier**: open, unblocked, unclaimed child tickets, ordered by ticket
  number.
- **Resolve**: append the resolution under `## Answer`, set
  `Status: resolved`, and append a one-line linked gist to the map's
  `## Decisions so far`.
- **Fog graduation**: remove a newly specifiable item from
  `## Not yet specified`, create its child ticket, then wire blocking in a
  second pass.
- **Out of scope**: close a mis-scoped ticket and link its scope decision from
  `## Out of scope`, not `## Decisions so far`.

Local Markdown has no native dependency relationship, so `Blocked by:` is the
authoritative fallback.

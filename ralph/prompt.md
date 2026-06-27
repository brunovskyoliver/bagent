# ISSUES

Local issue files from `issues/` are provided at start of context. Parse them to understand the open issues.

Work on **AFK issues only** — issues that are fully specified and need no human input. Skip anything marked HITL or "needs info".

You've also been passed the last few commits. Review them to understand what's been done.

If all AFK tasks are complete, output `<promise>NO MORE TASKS</promise>`.

# TASK SELECTION

Pick ONE task. Priority:

1. Critical bugfixes
2. Dev infrastructure (CI, types, test helpers) — unblocks everything else
3. Tracer bullets for new features — thin end-to-end slices through all layers
4. Polish and quick wins
5. Refactors

# EXPLORATION

Explore the repo before touching anything. Read `CLAUDE.md` for architecture and commands.

# IMPLEMENTATION

Use /tdd to complete the task.

# FEEDBACK LOOPS

Before committing, run:

```bash
cargo test --workspace
```

For type/build verification:

```bash
cargo build --workspace
```

Fix all failures before committing.

# COMMIT

Commit message must include:
1. Key decisions made
2. Files/crates changed
3. Blockers or notes for next iteration

# THE ISSUE

If the task is complete, move the issue file to `issues/done/`.

If not complete, append a progress note to the issue file.

# FINAL RULES

ONLY WORK ON A SINGLE TASK.

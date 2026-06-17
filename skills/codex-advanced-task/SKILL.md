---
name: codex-advanced-task
description: Dispatch complex cross-source business/admin tasks to the Codex external reasoning harness. Use only when the task rater returns CodexRecommended or CodexRequired — never for simple lookups, single-source summaries, drafts, translations, or one-step actions.
version: 1
risk: high
allowed_tools:
  - codex.run_task
tags:
  - codex
  - reconciliation
  - cross-source
  - brief
  - digest
  - dispute
  - contradiction
  - business-analysis
---

# Codex Advanced Task Skill

## When to use Codex

Codex is an **external advanced reasoning harness**, not a replacement for the local model.
Only invoke it when a task genuinely requires cross-source correlation or deep multi-step
reasoning that Ollama cannot reliably complete locally.

**Use Codex for:**
- Reconciling information across 3+ sources (Mail + Odoo + Notes + Files + WhatsApp)
- Building comprehensive client briefings from multiple data sources
- Investigating disputes or contradictions between records
- Weekly/monthly digests requiring data from multiple connectors
- Detecting timeline conflicts or missing follow-ups across sources
- Complex multi-step action planning with dependencies across systems
- Bulk operations that need reasoning before any action (e.g. draft 20 overdue-invoice replies)

**Do NOT use Codex for:**
- Simple one-source lookups ("čo hovorí tento email?")
- Short drafts or translations ("prepi toto po anglicky")
- Single-file reads or searches
- Opening apps, files, or URLs
- Anything the local Ollama model can answer in one step
- Tasks that do not require external reasoning (score < 60)

## Privacy guarantees

Codex receives **only a daemon-built context packet** — curated summaries and record
references approved by the user before dispatch. It never receives:

- Raw email or WhatsApp bodies (unless explicitly approved)
- Memory database contents or conversation history
- Odoo credentials or API tokens
- Keychain entries, passwords, `.ssh`, `.gnupg`
- Browser stores or cookies
- `~/Library/Application Support/bagent/` contents
- Screenshots beyond what is explicitly included in the packet
- Unrelated private files outside the selected scope

## Approval requirement

Every Codex invocation requires **explicit user approval** of the context packet before
dispatch. The daemon shows an approval modal with:
- The task description and complexity rating
- The list of context items (summaries + record refs) to be sent
- Privacy risk level
- Warning that Codex is an external service

Denial at any point cancels the run. There is no automatic approval path.

## Output contract

Codex returns a structured JSON result containing:
- `summary` — executive summary of findings
- `findings` — list of key facts discovered
- `conflicts` — detected contradictions or inconsistencies
- `proposed_actions` — suggested next steps (proposals only, not auto-executed)
- `drafts` — any draft text (emails, messages, reports)
- `questions_for_user` — clarifications needed before acting

**Codex cannot perform side effects.** All proposed actions flow back into bagent's
normal approval/tools framework and require separate user confirmation before execution.

## Rules engine

The `codex.run_task` tool is set to `Ask` level in `rules.yaml`. This cannot be changed
to `Auto` — Codex is always approval-gated at the rules level in addition to the per-run
context packet approval.

## Fallback behavior

If the `codex` binary is not found or returns an error:
- Report gracefully with the error from the daemon
- Suggest verifying the binary path in Settings → Codex
- Offer to attempt the task with the local Ollama model instead (with caveats about quality)

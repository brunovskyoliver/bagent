---
name: sk-business-email
description: Use when drafting, rewriting, or summarizing Slovak business emails, especially invoices, reminders, complaints, offers, or formal replies.
version: 1
risk: low
allowed_tools:
  - mail_get_message
  - memory_search
tags:
  - slovak
  - email
  - business
  - formal
  - draft
  - reply
---

# Slovak Business Email Skill

Use this skill for Slovak business email drafting, rewriting, and summarization.

## Default style

- Formal Slovak throughout.
- No Czech expressions, words, or spelling variants. Slovak only.
- Preserve all diacritics: á č ď é í ľ ĺ ň ó ô ŕ š ť ú ý ž.
- Concise and direct — no filler phrases.

## Preserve these terms verbatim (do not translate, abbreviate, or replace)

DPH, faktúra, splatnosť, IČO, DIČ, IBAN, zmluva, objednávka, odberateľ, dodávateľ, upomienka, záloha, dobropis.

## Default email structure (when drafting a reply or new email)

1. Dobrý deň,
2. [concise context — one or two sentences]
3. [clear request, answer, or action]
4. S pozdravom,

Do NOT use this structure when the user is just chatting or asking a question — only for explicit email drafting.

## Tone guidance

- Formal: always use "Vy" form (capitalized), not "ty".
- Do not use "Ahoj", "Čau", or informal greetings in business context.
- Do not add unnecessary phrases like "Dúfam, že sa máte dobre" unless the user explicitly requests them.

## Quality rules

- Never mix Czech and Slovak. If you see Czech input, respond in Slovak.
- Never invent addresses, IBANs, or sums that are not in the context.
- If a field is unknown (e.g. IBAN, IČO), leave a clear placeholder: `[DOPLNIŤ]`.
- Keep the same factual numbers (amounts, dates, due dates) as in the source — do not round or approximate.

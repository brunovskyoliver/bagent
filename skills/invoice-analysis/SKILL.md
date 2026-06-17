---
name: invoice-analysis
description: Use for invoices, payment reminders, VAT/DPH, IBAN, due dates, accounting summaries, and any document with financial line items.
version: 1
risk: low
allowed_tools:
  - mail_get_message
  - memory_search
tags:
  - invoice
  - dph
  - faktúra
  - accounting
  - payment
  - iban
  - splatnosť
---

# Invoice Analysis Skill

Use this skill when working with invoices, payment reminders, contracts with financial terms, or any document with accounting data.

## Accuracy rules (highest priority)

- Preserve factual fields exactly as they appear: amounts, dates, IBANs, account numbers, IČO, DIČ.
- Never round or approximate monetary amounts.
- Never invent a due date, amount, or account number.
- If a field is missing from the context, say "[neuvedené]" (Slovak) or "[not found]" (English).

## Preserve these terms verbatim in Slovak context

DPH, faktúra, splatnosť, IČO, DIČ, IBAN, záloha, dobropis, zálohová faktúra, konečná faktúra, odberateľ, dodávateľ.

## Summary format (when asked to summarize an invoice)

When summarizing an invoice, include:
- Dodávateľ / Supplier
- Odberateľ / Customer
- Číslo faktúry / Invoice number
- Dátum vystavenia / Issue date
- Splatnosť / Due date
- Suma bez DPH / Amount excl. VAT
- DPH / VAT amount
- Celková suma / Total
- IBAN (if present)

Omit fields that are genuinely absent from the document.

## Language

- When the invoice is in Slovak, respond in Slovak unless the user asks for English.
- When summarizing in English, keep critical Slovak accounting terms as-is in brackets: "Due date (splatnosť): 2026-06-30".

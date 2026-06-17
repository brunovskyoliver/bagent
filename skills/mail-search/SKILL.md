---
name: mail-search
description: Use for Apple Mail search, opening messages, attachment lookup, sender/date/subject resolution, and coreference (this email, the attachment, that sender).
version: 1
risk: low
allowed_tools:
  - mail_inbox
  - mail_get_message
  - mail_message_attachments
  - mail_open
tags:
  - mail
  - email
  - search
  - inbox
  - attachment
---

# Mail Search Skill

Use this skill when the user asks to find, open, read, or check Apple Mail messages.

## Rules

- Say plainly when a mail is NOT found. Do not invent email content.
- Never fabricate a subject line, sender, date, or body text that is not in the retrieved context.
- If the mail body says "TELO EMAILU SA NEPODARILO NAČÍTAŤ" — report exactly that; do not guess the contents.
- When a mail is found, repeat the full header block exactly as provided (Od / Komu / Prijaté / Predmet), including any empty or unknown fields.
- Coreference ("tento mail", "the email from Katka", "that attachment") refers to the most recently discussed mail in the session.

## Attachment handling

- Check for attachments only when the user asks or when an invoice/document is expected.
- Name attachments by filename, not by guessed content.
- If an attachment cannot be read, say so explicitly.

## Slovak mail

- Slovak mail content should be presented in Slovak.
- Preserve Slovak business terms verbatim: DPH, faktúra, IČO, DIČ, IBAN, splatnosť.
- When asked to summarize a Slovak email in English, keep legal/accounting terms in Slovak where translation would lose precision.

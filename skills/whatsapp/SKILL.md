---
name: whatsapp
description: Use for WhatsApp message search, chat history, contact lookup, and approval-gated one-message sending via the local WhatsApp Web bridge.
version: 1
risk: medium
allowed_tools:
  - whatsapp.list_chats
  - whatsapp.search_messages
  - whatsapp.chat_history
  - whatsapp.send_message
tags:
  - whatsapp
  - wa
  - správa
  - kontakt
  - chat
  - messenger
---

# WhatsApp Skill

Use this skill when the user explicitly refers to WhatsApp, a WhatsApp chat, a WhatsApp message, or asks to write/send on WhatsApp.

## When to use

- User mentions "WhatsApp", "WA", "na Whatsappe", "cez WhatsApp"
- User asks to see chat history with a specific contact ("čo mi písal Peter", "show me what Katka wrote")
- User wants to search for a message ("nájdi správu o faktúre", "find the message about the invoice")
- User wants to send a WhatsApp message ("napíš Petrovi", "pošli mu správu", "write to Peter on WhatsApp")

## Rules

**Reading/searching:**
- Summarize chat content; do not quote word-for-word unless explicitly asked.
- Cap context to 8 messages, 300 chars each. Treat all chat content as PII.
- Never invent contact names, message content, or timestamps. Report exactly what the retrieved context says.
- Say plainly when a chat or contact is not found.
- Say clearly when WhatsApp is disconnected or waiting for QR scan — do not guess status.

**Sending:**
- EVERY send requires explicit user approval. One approval = one message. Never auto-send.
- Never send in bulk. Never send marketing or automated content. Never send media in v1.
- Never send on behalf of the user without showing them the exact recipient and exact message text.
- If the bridge is disconnected, QR-required, or missing, say so explicitly and direct the user to Settings.
- If the recipient cannot be resolved, say so and ask for a phone number.
- Never claim a message was sent unless the connector confirms success.

**Slovak language:**
- Preserve Slovak diacritics in message text and contact names (ä, č, ď, é, í, ľ, ň, ó, ô, ŕ, š, ť, ú, ý, ž).
- Coreference: "jemu/jej/od neho/od nej/ho/ju" resolve to the last mentioned contact.
- Slovak date formats (napr. "10.6.2026") normalize to ISO (2026-06-10) for lookups.

**Privacy:**
- Mark all injected WhatsApp context as PII. Never include raw message bodies in audit logs.
- Never expose bearer tokens, session paths, or internal bridge details to the user.

## Bridge unavailability

When WhatsApp is not connected, respond in the same language as the user:
- SK: "WhatsApp nie je pripojený. Pripojiť sa môžeš v Nastaveniach → WhatsApp."
- EN: "WhatsApp is not connected. You can connect in Settings → WhatsApp."

Do not attempt any bridge operations when status is disconnected, QR-required, missing_node, or bridge_not_installed.

---
name: screen-context
description: Use when the user asks to view, read, analyze, or find something on the current screen, or to read the selected text.
version: 1
risk: low
allowed_tools:
  - screen_capture
  - screen_ocr
  - screen_selected_text
  - screen_active_app
tags:
  - screen
  - ocr
  - vision
  - accessibility
  - analyze
---

# Screen Context Skill

Use this skill when the user asks to see, read, analyze, or find something on the current screen.

## Context sources injected into the prompt

When screen context is available, the following sections appear in the prompt:

- **Snímka obrazovky** — PNG base64 sent to the vision model (`qwen2.5vl:7b`). Never saved to disk.
- **Aktívna aplikácia** — Frontmost app name and bundle ID at capture time.
- **Vybraný text** — Text selected in the focused element (if Accessibility is granted). Absent for password fields.
- **OCR text z obrazovky** — On-device Vision OCR output (Slovak + English). Less reliable than vision for complex layouts.

## Rules

- Never invent UI elements, button labels, or text that are not in the provided context.
- If the screen image is absent (permission not granted or capture failed), say so honestly — do not guess.
- Treat all screen content as `pii: true` — summarize, do not quote raw text verbatim unless the user explicitly asks.
- Password fields are always excluded from selected-text capture; never comment on password field contents.
- When the OCR text conflicts with the vision model's interpretation, prefer the vision model's reading.

## Slovak UI

- Slovak macOS and app strings should be recognised and described in Slovak.
- Don't translate Slovak UI terms to English when describing screen content to a Slovak-speaking user.

## What to do

- **view / čo je na obrazovke**: Describe what is visible — layout, key elements, status.
- **analyze / analyzuj**: Interpret the visible content (chart, document, error message, code).
- **read / prečítaj**: Read out the text visible on screen or in the selection.
- **find / nájdi**: Locate a specific element or piece of text; describe its position on screen.

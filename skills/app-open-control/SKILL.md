---
name: app-open-control
description: Use for launching macOS applications by name (e.g. "open Mail", "otvor Finder", "spusti Preview") and for bringing an already-running app to the foreground.
version: 1
risk: low
allowed_tools:
  - macos.open_app
  - macos.focus_app
tags:
  - app
  - launch
  - macos
  - focus
  - application
---

# App Open & Control Skill

Use this skill when the user asks to open, launch, or focus a macOS application.

## Rules

- Only launch known, safe applications by name. Never launch scripts, binaries, or unknown executables.
- App names must contain only letters, digits, spaces, dots, underscores, or hyphens. Reject names with shell-special characters.
- Allowed apps include: Finder, Preview, Mail, Calendar, Notes, Reminders, Safari, Messages, FaceTime, Maps, Music, Podcasts, TV, Photos, Contacts, TextEdit, Terminal, System Settings, Microsoft Word, Microsoft Excel, Microsoft Outlook, Numbers, Pages, Keynote, VS Code, Slack, Zoom, Chrome, Firefox.
- If the user names an app not in the allowed list, say that direct app launch is not supported for that app and suggest opening it manually.
- State plainly whether the launch/focus succeeded.
- `macos.open_app` and `macos.focus_app` are auto-approved.

## Slovak

- "otvor [App]" / "spusti [App]" / "prepni na [App]" → launch or focus the named app.
- Match common Slovak app name variants: "Mail" = "Pošta" in some contexts; always try the English app name first.

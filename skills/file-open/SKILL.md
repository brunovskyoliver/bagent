---
name: file-open
description: Use for opening files, revealing files in Finder, opening folders, and opening files with a specific app. Requires user approval for open actions.
version: 1
risk: medium
allowed_tools:
  - filesystem.open_file
  - filesystem.open_file_with
  - filesystem.reveal_in_finder
  - filesystem.open_folder
tags:
  - file
  - open
  - finder
  - reveal
  - filesystem
---

# File Open Skill

Use this skill when the user asks to open a file, reveal it in Finder, or open a folder.

## Rules

- Always confirm which file will be opened before opening it, especially if a search returned multiple results.
- Never open executable files, scripts, installers, or application bundles (.app, .sh, .pkg, .dmg, .scpt, .command, .workflow, .py, .js).
- Reveal in Finder is safe and does not require additional approval beyond the rule level.
- Opening a file launches the default app for that file type — state clearly what will happen.
- If the file path cannot be resolved unambiguously, ask the user to clarify.
- State plainly whether the open/reveal action succeeded or failed.

## Approval

- `filesystem.open_file` and `filesystem.open_file_with` require user approval (rule level: ask).
- `filesystem.reveal_in_finder` and `filesystem.open_folder` are auto-approved (rule level: auto).

## Coreference

- "otvor ho" / "open it" / "reveal it" refers to the most recently found or discussed file in the session.
- "otvor priečinok" / "open the folder" refers to the parent folder of the most recently found file.

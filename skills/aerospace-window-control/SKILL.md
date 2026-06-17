---
name: aerospace-window-control
description: Use for AeroSpace workspace and window management commands — switching desktops, moving/tiling/focusing windows, app focus.
version: 1
risk: low
allowed_tools:
  - aerospace_run
tags:
  - aerospace
  - window
  - workspace
  - desktop
  - tile
  - focus
---

# AeroSpace Window Control Skill

Use this skill when the user asks to switch workspaces, move windows, focus apps, or control the AeroSpace tiling window manager.

## Availability

This skill degrades gracefully when AeroSpace is not installed or not running. If the AeroSpace binary is not found, say so plainly — do not attempt to simulate or guess the action.

## Behavior rules

- All window actions remain governed by the rules engine (Auto / Ask / Forbidden).
- Do NOT perform side-effectful window actions without rule approval.
- When a workspace switch succeeds, confirm the action in a single short sentence (no more).
- When a command fails (workspace not found, binary missing), report the exact error.

## Common AeroSpace commands

- Switch to workspace N: `aerospace workspace N`
- Focus app: `aerospace focus --app <name>`
- Move window to workspace: `aerospace move-node-to-workspace N`
- List workspaces: `aerospace list-workspaces --all`

## Slovak input

Commands like "prepni na plochu 3", "prejdi na plochu 2", "fokus na terminál" map to AeroSpace workspace/focus commands. Respond in the user's language.

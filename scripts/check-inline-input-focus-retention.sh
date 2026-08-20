#!/usr/bin/env bash
set -euo pipefail

file="apps/macos/Sources/bagent/ChatView.swift"
fail=0

require() {
  local pattern="$1"
  local message="$2"
  if ! grep -Fq "$pattern" "$file"; then
    printf 'FAIL: %s\n' "$message"
    fail=1
  fi
}

require "@State private var inlineFocusRetryID" "inline input has no focus retry token"
require "keepInlineInputFocused(reason:" "inline input has no focus-retention helper"
require ".onChange(of: inputFocused)" "inline input does not react when focus is lost"
require "guard !focused else { return }" "inline input may refocus unnecessarily while already focused"
require "viewModel.notchInteractionMode == .input" "inline refocus is not gated to input mode"
require "NSApp.activate(ignoringOtherApps: true)" "inline refocus does not reactivate bagent after AppKit focus loss"

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

printf 'PASS: inline input keeps focus while input mode is visible\n'

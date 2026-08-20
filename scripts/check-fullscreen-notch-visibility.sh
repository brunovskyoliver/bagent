#!/usr/bin/env bash
set -euo pipefail

file="apps/macos/Sources/bagent/NotchWindowController.swift"
fail=0

require() {
  local pattern="$1"
  local message="$2"
  if ! grep -Fq "$pattern" "$file"; then
    printf 'FAIL: %s\n' "$message"
    fail=1
  fi
}

reject_in() {
  local start="$1"
  local end="$2"
  local pattern="$3"
  local message="$4"
  if sed -n "/$start/,/$end/p" "$file" | grep -Fq "$pattern"; then
    printf 'FAIL: %s\n' "$message"
    fail=1
  fi
}

require "private var statusPanelAllowedOverFullscreen: Bool" "fullscreen status-panel policy is missing"
require "case .input, .output:" "fullscreen policy no longer allows explicit input/output surfaces"
require "case .collapsed, .thinking:" "fullscreen policy no longer suppresses idle/thinking surfaces"
require "visibilityCancellable = chatViewModel.\$notchInteractionMode" "notch mode changes do not reapply fullscreen visibility"
require "chatViewModel.notchInteractionMode != .output" "fullscreen update can still collapse output surfaces"

reject_in "private func collapseInputForThinking()" "func collapse()" "statusPanel.orderFront(nil)" \
  "thinking transition still bypasses fullscreen visibility policy"

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

printf 'PASS: fullscreen notch visibility policy is covered\n'

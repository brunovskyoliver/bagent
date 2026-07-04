#!/usr/bin/env bash
set -euo pipefail

fail=0

reject() {
  local file="$1"
  local pattern="$2"
  local message="$3"
  if grep -Fq "$pattern" "$file"; then
    printf 'FAIL: %s\n' "$message"
    fail=1
  fi
}

reject apps/macos/Sources/bagent/ChatView.swift "textOverflowThreshold" "Spotlight input still has width-based overflow threshold"
reject apps/macos/Sources/bagent/ChatView.swift "promoteToChat(preserving:" "Spotlight input still promotes long text to chat"
reject apps/macos/Sources/bagent/ChatView.swift "pendingPromotedText" "Expanded chat still has promoted-draft reset/restore path"
reject apps/macos/Sources/bagent/ChatView.swift "promotedFocusRetryID" "Expanded chat still carries promoted-draft focus retry"
reject apps/macos/Sources/bagent/ChatViewModel.swift "promoteSpotlightDraft" "View model still exposes Spotlight promotion"
reject apps/macos/Sources/bagent/ChatViewModel.swift "pendingPromotedText" "View model still stores promoted draft"
reject apps/macos/Sources/bagent/ChatViewModel.swift "isPromotingSpotlightDraft" "View model still blocks send for promotion"
reject apps/macos/Sources/bagent/ChatViewModel.swift "onPromoteToChat" "View model still routes promotion to AppKit"
reject apps/macos/Sources/bagent/NotchWindowController.swift "promoteInputToChat" "Window controller still promotes input to chat"
reject apps/macos/Sources/bagent/NotchWindowController.swift "onPromoteToChat" "Window controller still registers promotion callback"

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

printf 'PASS: input has no long-text overflow promotion/reset path\n'

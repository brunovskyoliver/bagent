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

require "@State private var inlineContentOpacity" "non-notch inline content has no animation-gated opacity"
require "updateInlineContentOpacity(active: isInlineActive && !isVoice)" "non-notch inline opacity is not driven by interaction state"
require ".clipShape(nonNotchSurfaceShape)" "non-notch inline content is not clipped to the animated surface"
require "InlineNotchContent(viewModel: viewModel, showsInputLeadingIcon: false)" "non-notch input still renders the leading input source icon"
require "let showsInputLeadingIcon: Bool" "inline input content cannot vary leading icon by display style"
require "if showsInputLeadingIcon {" "inline input source icon is not conditional"

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

printf 'PASS: non-notch inline pill content is clipped, opacity-gated, and input iconless\n'

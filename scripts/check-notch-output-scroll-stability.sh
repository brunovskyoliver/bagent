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

reject() {
  local pattern="$1"
  local message="$2"
  if grep -Fq "$pattern" "$file"; then
    printf 'FAIL: %s\n' "$message"
    fail=1
  fi
}

require "static let contentVerticalInset" "output viewport inset is still a magic inline value"
require "class UserTrackingScrollView" "user scroll intent is not detected via a scrollWheel override"
require "func handleUserScroll" "coordinator has no single source of truth for user scroll intent"
require "clipHeightChanged" "output scroll view does not distinguish resize-driven bounds changes from user scroll"
require "outputHeightOnlyResize" "output bridge height-only resize can still recreate output content"
require "static var maxViewportHeight" "output scroll has no centralized fully-grown viewport height"
require "func viewportFullyGrown" "output scroll cannot tell growth phase from scroll phase"
require "func applyAutoScroll" "output scroll has no single two-phase auto-scroll policy"

# Growth phase must keep the text top-anchored; only a fully grown viewport may bottom-pin.
# acceptance-assertion: bottom pin requires fully grown viewport
if ! sed -n '/func applyAutoScroll/,/^        }/p' "$file" \
  | grep -Fq 'if viewportFullyGrown()'; then
  printf 'FAIL: auto-scroll bottom-pins without checking the viewport growth phase\n'
  fail=1
fi

reject "scrollToTop()" "streaming output still performs instant top-origin correction"
reject "shouldHoldTopWhileSurfaceGrows" "streaming output still uses top-hold logic instead of bottom pin"
reject "suppressScrollObservation" "streaming output still uses the racy scroll-observation suppression flag"
reject "requestBottomPin" "streaming output still uses async/animated bottom pins that caused shake"
reject "NSAnimationContext.runAnimationGroup" "streaming output still animates scroll pins (restarted animations shake)"

# The clip-resize handler must never override user scroll intent.
# acceptance-assertion: resize preserves user scroll intent
if sed -n '/@objc func boundsDidChange/,/^        }/p' "$file" \
  | grep -Fq 'userScrolledAway = false'; then
  printf 'FAIL: resize-driven bounds changes still reset user scroll intent\n'
  fail=1
fi

# acceptance-assertion: old hard-coded viewport inset is absent
if sed -n '/private struct LatestAssistantOutputScrollView/,/private extension ChatMessage/p' "$file" \
  | grep -Fq 'height: contentBridge - 14'; then
  printf 'FAIL: output scroll viewport still uses the old hard-coded inset\n'
  fail=1
fi

# acceptance-assertion: height-only resize preserves output content identity
if sed -n '/let surfaceTargetChanged/,/withAnimation(surfaceAnimation)/p' "$file" \
  | grep -Fq 'inlineRevealID = UUID()' \
  && ! sed -n '/let surfaceTargetChanged/,/withAnimation(surfaceAnimation)/p' "$file" \
    | grep -Fq 'if !outputHeightOnlyResize'; then
  printf 'FAIL: output bridge resize can still recreate inline output scroll view\n'
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

printf 'PASS: notch output scroll stability is covered\n'

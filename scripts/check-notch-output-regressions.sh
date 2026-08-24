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

require() {
  local file="$1"
  local pattern="$2"
  local message="$3"
  if ! grep -Fq "$pattern" "$file"; then
    printf 'FAIL: %s\n' "$message"
    fail=1
  fi
}

reject apps/macos/Sources/bagent/ChatView.swift 'Text("Streaming")' "notch output still renders Streaming status title"
reject apps/macos/Sources/bagent/ChatView.swift 'viewModel.isLatestAssistantStreaming ? 56 : 76' "notch output still uses streaming/final fixed text heights"
require apps/macos/Sources/bagent/ChatView.swift "outputMaxBridgeHeight" "notch output has no max dynamic bridge height"
require apps/macos/Sources/bagent/ChatView.swift "estimatedNotchTextHeight" "notch output height is not based on text measurement"
require apps/macos/Sources/bagent/ChatView.swift "NotchOutputLayout.bottomSlack" "notch output has no explicit bottom slack for the final line"
require apps/macos/Sources/bagent/ChatView.swift "refreshOutputSurfaceIfNeeded" "notch output resize is not threshold-gated"
require apps/macos/Sources/bagent/ChatView.swift "syntheticNotchWidth" "external/non-notch fake notch has no measured synthetic notch width"
require apps/macos/Sources/bagent/NotchWindowController.swift "screen.visibleFrame.maxY" "fake notch menu-bar height does not use actual screen visible frame"
reject apps/macos/Sources/bagent/ChatView.swift "fakeNotchStatusPos" "collapsed fake notch status dot is centered in fake notch gap instead of right-side status slot"
reject apps/macos/Sources/bagent/ChatView.swift '.animation(surfaceAnimation, value: viewModel.streamingChunk)' "notch/non-notch output still animates the whole surface on every streamed token"
reject apps/macos/Sources/bagent/ChatView.swift 'ceil(usedRect.height) + 2' "latest assistant output text view still has only 2pt bottom slack"
reject apps/macos/Sources/bagent/ChatView.swift '.init(color: .white.opacity(0.18), location: 1.0)' "notch output still fades/clips the final line at the bottom edge"
reject apps/macos/Sources/bagent/ChatView.swift 'struct MenuBarPillView' "external/non-notch status surface still has the old rounded menu-bar pill branch"
reject apps/macos/Sources/bagent/ChatView.swift 'let isOnNotch: Bool' "status pill still branches physical notch vs non-notch instead of always rendering notch-style surface"
reject apps/macos/Sources/bagent/NotchWindowController.swift 'menuBarBottomY - NotchWrapMetrics.inlineBridgeHeight' "non-notch fake notch frame can float below menu bar instead of staying top-flush"
reject apps/macos/Sources/bagent/NotchWindowController.swift 'voicePanel' "external/non-notch voice still uses separate voice panel instead of inline fake-notch bridge"
reject docs/UI_DESIGN.md 'Transparent pill inside menu bar' "UI docs still define external/non-notch idle as pill"

# acceptance-assertion: streaming chunks do not directly refresh the surface
if sed -n '/\.onChange(of: viewModel\.streamingChunk)/,/^        }/p' apps/macos/Sources/bagent/ChatView.swift | grep -Fq "refreshSurface()"; then
  printf 'FAIL: notch output still directly refreshes surface on every streaming chunk\n'
  fail=1
fi

# acceptance-assertion: fill and hit testing share the top-flat shape
if ! sed -n '/ZStack(alignment: \.topLeading)/,/\.fill(\.black)/p' apps/macos/Sources/bagent/ChatView.swift | grep -Fq "NotchWrapShape("; then
  printf 'FAIL: notch output fill does not use the same top-flat shape as hit testing\n'
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

printf 'PASS: notch output layout regressions covered\n'

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

require "private var animatedInlineStatusPos" \
  "inline thinking/output status dot is not positioned from animated surface geometry"
require "private var inlineSurfaceLayer" \
  "inline output chrome/content is not grouped into one animated layer"
require "private var statusDotLayer" \
  "status dot is not rendered as a single persistent layer"
require "private var showsLeftStatusIcon" \
  "left notch icon has no centralized visibility rule"
require "viewModel.notchInteractionMode != .thinking" \
  "collapsed thinking state can still show the left notch icon"
require "private var collapsedStatusPos" \
  "thinking status dot has no collapsed idle start position"
require "@State private var returningStatusDotFromOutput" \
  "status dot has no reverse travel state for output collapse"
require "@State private var outputStatusReturnStartPos" \
  "status dot does not preserve the output target before collapse"
require "@State private var outputStatusReturnStartBridgeHeight" \
  "status dot collapse progress is not anchored to the output bridge height"
require "private var outputStatusTravelProgress" \
  "thinking-to-output status dot is not driven by expand animation progress"
require "private var statusDotPos" \
  "status dot has no unified animated position"
require "private func updateStatusDotTravelState()" \
  "status dot travel state is not updated across output mode transitions"
require "if previousNotchInteractionMode == .output && currentMode != .output" \
  "leaving output mode does not start reverse status-dot travel"
require "bridgeHeight / startBridgeHeight" \
  "collapse status-dot movement is not synchronized to bridge shrink progress"
require "previousNotchInteractionMode == .output && viewModel.notchInteractionMode != .output" \
  "status dot can snap for one frame before reverse travel state is saved"
require ".clipShape(animatedNotchClipShape)" \
  "inline output layer is not clipped to the animated notch shape"
require "StatusDotView(status: status, pulsing: \$pulsing, reduceMotion: reduceMotion, copyFlashed: copyFlashed, isDragTargeted: isDragTargeted)" \
  "inline thinking/output status dot does not reuse the blinking status dot"
require "viewModel.notchInteractionMode == .input || viewModel.notchInteractionMode == .output" \
  "thinking mode still counts as an expanded inline surface"

reject_in "private var inlineStatusPos" "private func isNearVisibleSurface" "inlineBridgeHeight(for: viewModel.notchInteractionMode)" \
  "inline status dot can still jump to target output geometry before the surface grows"
reject_in "private var inlineSurfaceLayer" "private func isNearVisibleSurface" "LightbulbStatusDotView" \
  "inline thinking/output still renders a second non-blinking lightbulb dot"
reject_in "private var inlineSurfaceLayer" "private var statusDotLayer" "StatusDotView" \
  "inline surface can still render a duplicate status dot"
reject_in "var body: some View" "Voice content in bridge area" "if !isInlineActive && (viewModel.isExpanded || isHovered || isVoiceActive)" \
  "left notch icon still keys directly off stale expanded state"

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

printf 'PASS: thinking-to-output status dot is tied to the animated notch layer\n'

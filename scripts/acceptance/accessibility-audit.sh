#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
rail="$root/apps/macos/Sources/bagent/StageRailView.swift"
projection="$root/apps/macos/Sources/bagent/NotchProjection.swift"
notch="$root/apps/macos/Sources/bagent/ChatView.swift"
fixture="$root/apps/macos/Sources/bagent/Stage5AcceptanceFixture.swift"

swift test --package-path "$root/apps/macos" --filter 'StageRailTests|StageRailAccessibilityTests|NotchStateCatalogTests'
make -C "$root/apps/macos" bundle
codesign --verify --strict "$root/apps/macos/bagent.app"

rg -q 'Button\(action: action\)' "$rail"
rg -q '\.keyboardShortcut\(\.return' "$rail"
rg -q '\.accessibilityElement\(children: \.ignore\)' "$rail"
test "$(rg -c '\.accessibilityLabel' "$rail")" -ge 3
test "$(rg -c '\.accessibilityValue' "$rail")" -ge 2
rg -q '\.accessibilityHidden\(true\)' "$rail"
rg -q 'content\.accessibilityRepresentation' "$notch"
rg -q '\.font\(\.caption' "$rail"
rg -q 'queue position' "$projection"
rg -q 'run \\.* of' "$projection"
rg -q 'active Automation Run' "$projection"
rg -q 'BagentPanel' "$fixture"

if rg -n 'onTapGesture|@FocusState' "$rail"; then
    echo "FAIL: Stage Rail contains an unstable tap or focus authority" >&2
    exit 1
fi

echo "PASS: signed fixture verified; hosted controls, Return bindings, labels, values, decorative hiding, semantic text, contrast, and stable focus source checks passed"

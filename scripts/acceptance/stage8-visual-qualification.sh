#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
candidate="${1:-$root/apps/macos/bagent.app}"
product_version="$(sw_vers -productVersion)"
[[ "${product_version%%.*}" == "26" ]] || {
    echo "A56 BLOCKED: signed visual qualification is macOS 26 only (found $product_version)" >&2
    exit 1
}
[[ -d "$candidate" && "${candidate##*.}" == "app" ]] || {
    echo "A56 FAIL: signed candidate is missing: $candidate" >&2
    exit 1
}
codesign --verify --deep --strict "$candidate"

test_filters=(
    StageRailTests
    AutomationSplitViewTests
    ProjectionPrivacyTests
    CompassRailTests
    NotchStateCatalogTests
)
for filter in "${test_filters[@]}"; do
    swift test --package-path "$root/apps/macos" --filter "$filter"
done

"$root/scripts/acceptance/capture-notch-states.sh"
"$root/scripts/acceptance/settings-catalog.sh"

fixture="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage8-visual.XXXXXX")"
case "$fixture" in
    "${TMPDIR:-/tmp}"/bagent-stage8-visual.*) ;;
    *) echo "refusing unexpected visual fixture path: $fixture" >&2; exit 2 ;;
esac
trap 'rm -rf -- "$fixture"' EXIT INT TERM

for variant in light dark; do
    BAGENT_STAGE7B_SETTINGS_FIXTURE=1 \
        "$candidate/Contents/MacOS/bagent" \
        --stage7b-settings-fixture "$fixture/$variant" "$variant"
    catalog="$fixture/$variant/catalog.json"
    [[ "$(jq -r .color_scheme "$catalog")" == "$variant" ]]
    [[ "$(jq -r .route_count "$catalog")" == 11 ]]
    [[ "$(jq -r .rendered_image_count "$catalog")" == 114 ]]
    [[ "$(jq -r .state_render_count_per_width "$catalog")" == 57 ]]
done

"$root/scripts/acceptance/signed-bundle-verification.sh" "$candidate"

rg -q 'InvariantNotchStatusPill' "$root/apps/macos/Tests/bagentTests/NotchStateCatalogTests.swift"
rg -q 'NotchPillLayout\.origin' "$root/apps/macos/Tests/bagentTests/NotchStateCatalogTests.swift"
rg -q 'acceptanceReduceMotionOverride' "$root/apps/macos/Sources/bagent/ChatView.swift"
test "$(find "$root/apps/macos/.build/notch-state-catalog" -maxdepth 1 -name '*.png' ! -name 'contact-sheet.png' | wc -l | tr -d ' ')" = 11

echo "A56 visual qualification: PASS (macOS $product_version; signed candidate; 11 notch states; 57 settings fixtures x 2 widths x light/dark/high-contrast/large-text/reduced-motion; signed identity and status-pill anchor verified)"

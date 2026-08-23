#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
candidate="${1:-$root/apps/macos/bagent.app}"
baselines="$root/scripts/acceptance/stage8-visual-baselines.json"
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
[[ "$(jq -r .macos_major "$baselines")" == "${product_version%%.*}" ]]

aggregate_png_sha() {
    local directory="$1" depth="$2"
    (
        cd "$directory"
        find . -maxdepth "$depth" -type f -name '*.png' ! -name 'contact-sheet.png' -print0 |
            LC_ALL=C sort -z |
            xargs -0 shasum -a 256
    ) | shasum -a 256 | awk '{print $1}'
}

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

"$root/scripts/acceptance/settings-catalog.sh"

fixture="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage8-visual.XXXXXX")"
case "$fixture" in
    "${TMPDIR:-/tmp}"/bagent-stage8-visual.*) ;;
    *) echo "refusing unexpected visual fixture path: $fixture" >&2; exit 2 ;;
esac
trap 'rm -rf -- "$fixture"' EXIT INT TERM

transition_evidence="$fixture/notch-transition.json"
BAGENT_NOTCH_CAPTURE_DIR="$fixture/notch-state-catalog" \
BAGENT_NOTCH_TRANSITION_EVIDENCE="$transition_evidence" \
    "$root/scripts/acceptance/capture-notch-states.sh" "$candidate"
[[ "$(jq -r .status "$transition_evidence")" == pass ]]
[[ "$(jq -r .automation_split_view_capture_count "$transition_evidence")" == 2 ]]
[[ "$(jq -r .transition_count "$transition_evidence")" =~ ^[2-9][0-9]*$ ]]
for field in recorded_transition_frame_count distinct_transition_frame_count normal_motion_frame_count reduced_motion_frame_count interruptions_injected interruptions_reconciled; do
    count="$(jq -r ".$field" "$transition_evidence")"
    [[ "$count" =~ ^[1-9][0-9]*$ ]] || {
        echo "A56 transition evidence is zero: $field" >&2
        exit 1
    }
done
[[ "$(jq -r .interruptions_injected "$transition_evidence")" == "$(jq -r .interruptions_reconciled "$transition_evidence")" ]]
[[ "$(jq -r .status_pill_anchor_invariant "$transition_evidence")" == true ]]
recorded_files="$(find "$fixture/notch-state-catalog/transition-frames" -type f -name '*.png' | wc -l | tr -d ' ')"
[[ "$recorded_files" == "$(jq -r .recorded_transition_frame_count "$transition_evidence")" ]]

for variant in default light dark large-text high-contrast reduce-motion; do
    BAGENT_STAGE7B_SETTINGS_FIXTURE=1 \
        "$candidate/Contents/MacOS/bagent" \
        --stage7b-settings-fixture "$fixture/$variant" "$variant"
    catalog="$fixture/$variant/catalog.json"
    if [[ "$variant" == light || "$variant" == dark ]]; then
        [[ "$(jq -r .color_scheme "$catalog")" == "$variant" ]]
    else
        [[ "$(jq -r .color_scheme "$catalog")" == system ]]
    fi
    [[ "$(jq -r .route_count "$catalog")" == 11 ]]
    [[ "$(jq -r .rendered_image_count "$catalog")" == 114 ]]
    [[ "$(jq -r .state_render_count_per_width "$catalog")" == 57 ]]
done

notch_hash="$(aggregate_png_sha "$fixture/notch-state-catalog" 1)"
split_hash="$(aggregate_png_sha "$fixture/notch-state-catalog/automation-split-view" 1)"
[[ "$notch_hash" == "$(jq -r .notch_states "$baselines")" ]] || {
    echo "A56 FAIL: notch state catalog differs from the approved baseline ($notch_hash)" >&2
    exit 1
}
[[ "$split_hash" == "$(jq -r .automation_split_view "$baselines")" ]] || {
    echo "A56 FAIL: Automation Split View differs from the approved baseline ($split_hash)" >&2
    exit 1
}
for variant in default light dark large-text high-contrast reduce-motion; do
    settings_hash="$(aggregate_png_sha "$fixture/$variant" 1)"
    [[ "$settings_hash" == "$(jq -r --arg variant "$variant" '.settings[$variant]' "$baselines")" ]] || {
        echo "A56 FAIL: $variant Compass Rail/settings catalog differs from the approved baseline ($settings_hash)" >&2
        exit 1
    }
done

"$root/scripts/acceptance/signed-bundle-verification.sh" "$candidate"

rg -q 'InvariantNotchStatusPill' "$root/apps/macos/Tests/bagentTests/NotchStateCatalogTests.swift"
rg -q 'NotchPillLayout\.origin' "$root/apps/macos/Tests/bagentTests/NotchStateCatalogTests.swift"
rg -q 'acceptanceReduceMotionOverride' "$root/apps/macos/Sources/bagent/ChatView.swift"
test "$(find "$fixture/notch-state-catalog" -maxdepth 1 -name '*.png' ! -name 'contact-sheet.png' | wc -l | tr -d ' ')" = 11

echo "A56 visual qualification: PASS (macOS $product_version; signed candidate; approved hashes matched for 11 notch states, 2 Automation Split View states, and 57 settings/Compass Rail fixtures x 2 widths x 6 variants; $(jq -r .recorded_transition_frame_count "$transition_evidence") hosted transition PNG frames with $(jq -r .distinct_transition_frame_count "$transition_evidence") distinct renders; $(jq -r .interruptions_reconciled "$transition_evidence") mid-transition interruptions reconciled; signed identity and status-pill anchor verified)"

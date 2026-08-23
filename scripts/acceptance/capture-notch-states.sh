#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
candidate="${1:-$root/apps/macos/bagent.app}"
catalog="${BAGENT_NOTCH_CAPTURE_DIR:-$root/apps/macos/.build/notch-state-catalog}"
evidence="${BAGENT_NOTCH_TRANSITION_EVIDENCE:-$catalog/evidence.json}"
sources="$root/apps/macos/Sources/bagent"

mkdir -p "$catalog"
codesign --verify --deep --strict "$candidate"
BAGENT_STAGE8_VISUAL_FIXTURE=1 \
    "$candidate/Contents/MacOS/bagent" --stage8-visual-capture "$catalog" "$evidence"

expected=(idle queued loading thinking tool approval streaming completion failure cancellation interruption)
for state in "${expected[@]}"; do
    test -s "$catalog/$state.png"
done

test "$(find "$catalog" -maxdepth 1 -name '*.png' ! -name 'contact-sheet.png' | wc -l | tr -d ' ')" = 11
[[ "$(jq -r .status "$evidence")" == pass ]]
[[ "$(jq -r .rendered_notch_state_count "$evidence")" == 11 ]]
[[ "$(jq -r .transition_count "$evidence")" == 22 ]]
rg -q 'surfaceDuration: 0\.58' "$sources/NotchProjection.swift"
rg -q 'contentRevealDelay: 0\.36' "$sources/NotchProjection.swift"
rg -q 'duration: 0\.38' "$sources/StageRailView.swift"
rg -q 'duration: 0\.72' "$sources/StageRailView.swift"
rg -q 'duration: 1\.8' "$sources/StageRailView.swift"
rg -q 'guard !reduceMotion else' "$sources/StageRailView.swift"

if rg -n '\.(blur|shadow)\(|Material|material\)' \
    "$sources/StageRailView.swift" "$sources/NotchProjection.swift"; then
    echo "FAIL: Stage Rail introduced material, blur, or shadow" >&2
    exit 1
fi

echo "PASS: signed candidate rendered 11 deterministic notch-state PNGs and executed 22 normal/reduced-motion transitions in $catalog"

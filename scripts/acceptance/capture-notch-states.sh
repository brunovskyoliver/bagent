#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
catalog="$root/apps/macos/.build/notch-state-catalog"
sources="$root/apps/macos/Sources/bagent"

mkdir -p "$catalog"
BAGENT_NOTCH_CAPTURE_DIR="$catalog" \
    swift test --package-path "$root/apps/macos" --filter NotchStateCatalogTests

expected=(idle queued loading thinking tool approval streaming completion failure cancellation interruption)
for state in "${expected[@]}"; do
    test -s "$catalog/$state.png"
done

test "$(find "$catalog" -maxdepth 1 -name '*.png' ! -name 'contact-sheet.png' | wc -l | tr -d ' ')" = 11
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

echo "PASS: rendered 11 deterministic notch-state PNGs in $catalog and verified accepted motion constants"

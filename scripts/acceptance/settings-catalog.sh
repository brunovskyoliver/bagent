#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
fixture=$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage7b-settings.XXXXXX")
trap 'rm -rf "$fixture"' EXIT

swift test --package-path "$root/apps/macos" --filter CompassRailTests
"$root/scripts/acceptance/settings-authority.sh"
catalog_assertions=0
catalog_assert() {
    "$@"
    catalog_assertions=$((catalog_assertions + 1))
}
catalog_assert rg -q 'static func route' "$root/apps/macos/Sources/bagent/CompassRail.swift"
catalog_assert rg -q '\.focused\(\$focusedControl' "$root/apps/macos/Sources/bagent/NotchSettingsContent.swift"
catalog_assert rg -q 'SettingsFontModifier' "$root/apps/macos/Sources/bagent/NotchSettingsContent.swift"
catalog_assert rg -q 'accessibilityAddTraits\(peer == area \? \.isSelected' "$root/apps/macos/Sources/bagent/NotchSettingsContent.swift"
catalog_assert rg -q 'compassRailHighContrast' "$root/apps/macos/Sources/bagent/Stage7BSettingsAcceptanceCLI.swift"
catalog_assert rg -q 'validateInteractionContract' "$root/apps/macos/Sources/bagent/Stage7BSettingsAcceptanceCLI.swift"
catalog_assert rg -q 'validateContrastContract' "$root/apps/macos/Sources/bagent/Stage7BSettingsAcceptanceCLI.swift"
make -C "$root/apps/macos" bundle
codesign --verify --strict "$root/apps/macos/bagent.app"

for variant in default large-text high-contrast reduce-motion; do
    BAGENT_STAGE7B_SETTINGS_FIXTURE=1 \
        "$root/apps/macos/bagent.app/Contents/MacOS/bagent" \
        --stage7b-settings-fixture "$fixture/$variant" "$variant"
done

for variant in default large-text high-contrast reduce-motion; do
    test -s "$fixture/$variant/catalog.json"
    test "$(find "$fixture/$variant" -maxdepth 1 -name '*.png' | wc -l | tr -d ' ')" = 114
    test "$(jq -r '.route_count' "$fixture/$variant/catalog.json")" = 11
    test "$(jq -c '.panel_widths' "$fixture/$variant/catalog.json")" = '[701,941]'
    test "$(jq -r '.pixel_height' "$fixture/$variant/catalog.json")" = 318
    test "$(jq -r '.rendered_image_count' "$fixture/$variant/catalog.json")" = 114
    test "$(jq -r '.state_render_count_per_width' "$fixture/$variant/catalog.json")" = 57
    test "$(jq -r '.model_runtime_state_count' "$fixture/$variant/catalog.json")" = 8
    test "$(jq -r '.model_runtime_fixture_count' "$fixture/$variant/catalog.json")" = 4
    test "$(jq -r '.validation_state_count' "$fixture/$variant/catalog.json")" = 5
    test "$(jq -r '.permission_state_count' "$fixture/$variant/catalog.json")" = 2
done

echo "settings catalog: 4 variants x 2 synthetic panel widths x 57 route/state fixtures, 456 rendered PNGs, $catalog_assertions keyboard/focus/accessibility assertions, runtime/validation/permission/high-contrast state matrix, signed bundle verified"

#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
candidate="${1:-$root/apps/macos/bagent.app}"
product_version="$(sw_vers -productVersion)"
[[ "${product_version%%.*}" == "26" ]] || {
    echo "A57 BLOCKED: signed accessibility qualification is macOS 26 only (found $product_version)" >&2
    exit 1
}
codesign --verify --deep --strict "$candidate"

swift test --package-path "$root/apps/macos" --filter CompassRailTests
swift test --package-path "$root/apps/macos" --filter CompassRailAccessibilityTests

fixture="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage8-accessibility.XXXXXX")"
case "$fixture" in
    "${TMPDIR:-/tmp}"/bagent-stage8-accessibility.*) ;;
    *) echo "refusing unexpected accessibility fixture path: $fixture" >&2; exit 2 ;;
esac
trap 'rm -rf -- "$fixture"' EXIT INT TERM

notch_evidence="$fixture/notch-ax.json"
BAGENT_STAGE8_ACCESSIBILITY_FIXTURE=1 \
    "$candidate/Contents/MacOS/bagent" \
    --stage8-accessibility-fixture "$notch_evidence"
[[ "$(jq -r .status "$notch_evidence")" == pass ]]
[[ "$(jq -r .accessibility_available "$notch_evidence")" == true ]]
[[ "$(jq -r .skipped_count "$notch_evidence")" == 0 ]]
for field in active_element_count approval_element_count assertion_count; do
    count="$(jq -r ".$field" "$notch_evidence")"
    [[ "$count" =~ ^[1-9][0-9]*$ ]] || { echo "A57 $field is zero" >&2; exit 1; }
done

settings_dir="$fixture/settings"
set +e
BAGENT_STAGE7B_AX_ACCEPTANCE_DIR="$settings_dir" \
    "$root/scripts/acceptance/settings-accessibility.sh" --prepare \
    >"$fixture/settings-prepare.log" 2>&1
prepare_status=$?
set -e
if [[ "$prepare_status" != 0 && "$prepare_status" != 78 ]]; then
    pregrant_evidence="$settings_dir/live-ax-evidence/live-ax.json"
    if [[ ! -s "$pregrant_evidence" ]] ||
       [[ "$(plutil -extract status raw -o - "$pregrant_evidence" 2>/dev/null || true)" != pass ]]; then
        cat "$fixture/settings-prepare.log" >&2
        echo "A57 FAIL: signed settings fixture preparation failed ($prepare_status)" >&2
        exit 1
    fi
    echo "A57 settings fixture preparation: live AX was already available; continuing with the signed run"
fi

BAGENT_STAGE7B_AX_ACCEPTANCE_DIR="$settings_dir" \
    "$root/scripts/acceptance/settings-accessibility.sh" --run
settings_evidence="$settings_dir/live-ax-evidence/live-ax.json"
[[ "$(plutil -extract status raw -o - "$settings_evidence")" == pass ]]
[[ "$(plutil -extract accessibility_available raw -o - "$settings_evidence")" == true ]]
[[ "$(plutil -extract skipped_count raw -o - "$settings_evidence")" == 0 ]]
for field in route_count element_count assertion_count; do
    count="$(plutil -extract "$field" raw -o - "$settings_evidence")"
    [[ "$count" =~ ^[1-9][0-9]*$ ]] || { echo "A57 settings $field is zero" >&2; exit 1; }
done

set +e
hosted_output="$(swift test --package-path "$root/apps/macos" --filter StageRailAccessibilityTests 2>&1)"
hosted_status=$?
set -e
[[ "$hosted_status" == 0 ]] || { printf '%s\n' "$hosted_output" >&2; exit 1; }
if printf '%s\n' "$hosted_output" | rg -qi 'skip|skipped'; then
    echo "A57 hosted XCTest Accessibility API check: SKIPPED (not PASS; live signed AX fixtures are the executed evidence)"
else
    echo "A57 hosted XCTest Accessibility API check: executed without a reported skip"
fi

echo "A57 accessibility qualification: PASS (macOS $product_version; signed live notch AX states=2, notch assertions=$(jq -r .assertion_count "$notch_evidence"), settings routes=$(plutil -extract route_count raw -o - "$settings_evidence"), settings assertions=$(plutil -extract assertion_count raw -o - "$settings_evidence"), skipped live assertions=0; no TCC mutation)"

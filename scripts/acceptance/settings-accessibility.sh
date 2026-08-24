#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
mode=${1:---prepare}
acceptance_dir=${BAGENT_STAGE7B_AX_ACCEPTANCE_DIR:-/tmp/bagent-stage7b-acceptance-34}
app="$acceptance_dir/settings-ax-fixture.app"
evidence="$acceptance_dir/live-ax-evidence"
binary="$app/Contents/MacOS/bagent"
plist="$app/Contents/Info.plist"
fixture_identifier="sk.bagent.stage7b.ax.fixture"
expected_team_id="QUB47S3XTF"
expected_identity="Apple Development: obrunovsky7@gmail.com (D63PW2838J)"
expected_requirement='identifier "sk.bagent.stage7b.ax.fixture" and anchor apple generic and certificate leaf[subject.CN] = "Apple Development: obrunovsky7@gmail.com (D63PW2838J)" and certificate 1[field.1.2.840.113635.100.6.2.1] /* exists */'

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

assert_static_contract() {
    local settings="$root/apps/macos/Sources/bagent/NotchSettingsContent.swift"
    local routes="$root/apps/macos/Sources/bagent/CompassRail.swift"
    local assertions=0
    check() {
        if ! "$@"; then
            fail "$*"
        fi
        assertions=$((assertions + 1))
    }

    check rg -q 'accessibilityLabel\("Compass Rail"\)' "$settings"
    check test "$(rg -c '\.accessibilityLabel\(peer\.title\)' "$settings")" = 1
    check test "$(rg -c 'ForEach\(CompassRailArea\.allCases' "$settings")" = 1
    check rg -q 'accessibilityAddTraits\(peer == area \? \.isSelected' "$settings"
    check rg -q 'accessibilityAddTraits\(\.isHeader\)' "$settings"
    check rg -q 'accessibilityLabel\(area\.backAccessibilityLabel\)' "$settings"
    check rg -q 'accessibilityHidden\(true\)' "$settings"
    check rg -q 'accessibilityReduceMotion' "$settings"
    check rg -q 'CompassRailRoute\.acceptedRoutes' "$routes"
    check rg -q 'enum CompassRailKeyboard' "$routes"
    check rg -q '\.focused\(\$focusedControl' "$settings"
    check rg -q 'SettingsFontModifier' "$settings"
    for area in general modelRuntime integrations privacyAndPermissions; do
        check rg -q "case \\.${area}" "$routes"
    done
    for child in whatsapp odoo codex fullDiskAccess screenRecording accessibility rulesAndApprovalPolicy; do
        check rg -q "case \\.${child}" "$routes"
    done

    if rg -n 'ScrollView|LazyV|TextEditor|rules\.yaml' "$settings"; then
        fail "settings accessibility fixture found scrolling or raw policy editing"
    fi
    echo "settings accessibility: deterministic contract and $assertions source assertions verified"
}

build_fixture() {
    [[ ! -e "$app" ]] || fail "refusing to overwrite the stable fixture at $app"
    mkdir -p "$acceptance_dir"
    swift build --package-path "$root/apps/macos" -c release
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$root/apps/macos/.build/release/bagent" "$binary"
    cp "$root/apps/macos/Info.plist" "$plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $fixture_identifier" "$plist"
    /usr/libexec/PlistBuddy -c 'Set :CFBundleName Stage 7B Accessibility Fixture' "$plist"
    /usr/libexec/PlistBuddy -c 'Add :CFBundleDisplayName string Stage 7B Accessibility Fixture' "$plist"

    security find-identity -v -p codesigning | rg -Fq "\"$expected_identity\"" || fail "expected Apple Development identity is unavailable"
    local identity="$expected_identity"
    codesign --force --sign "$identity" --timestamp=none "$binary"
    codesign --force --sign "$identity" --timestamp=none "$app"
}

verify_fixture_identity() {
    codesign --verify --deep --strict "$app"
    local identifier
    identifier=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$plist")
    [[ "$identifier" == "$fixture_identifier" ]] || fail "unexpected fixture identifier: $identifier"
    local team_id
    team_id=$(codesign -dv --verbose=4 "$app" 2>&1 | sed -n 's/^TeamIdentifier=//p' | head -n 1)
    [[ "$team_id" == "$expected_team_id" ]] || fail "unexpected fixture Team ID: $team_id"
    local requirement
    requirement=$(codesign -d -r- "$app" 2>&1 | sed -n 's/^designated => //p')
    [[ "$requirement" == "$expected_requirement" ]] || fail "unexpected designated requirement: $requirement"
    printf '%s\n' "fixture=$app" "bundle_identifier=$identifier" "team_id=$team_id" "designated_requirement=$requirement"
}

run_fixture() {
    mkdir -p "$evidence"
    BAGENT_STAGE7B_SETTINGS_AX_FIXTURE=1 "$binary" --stage7b-settings-ax-fixture "$evidence"
}

read_count() {
    plutil -extract "$1" raw -o - "$evidence/live-ax.json"
}

case "$mode" in
    --prepare)
        build_fixture
        verify_fixture_identity
        assert_static_contract
        set +e
        run_fixture
        status=$?
        set -e
        [[ "$status" == 78 ]] || fail "fixture did not report the expected unavailable Accessibility state (exit $status)"
        plutil -extract status raw -o - "$evidence/live-ax.json" | rg -qx 'needs_accessibility_grant' || fail "unexpected pregrant fixture status"
        plutil -extract accessibility_available raw -o - "$evidence/live-ax.json" | rg -qx 'false' || fail "pregrant fixture did not report Accessibility unavailable"
        echo "STAGE 7B signed live-AX fixture prepared"
        echo "fixture_path=$app"
        echo "continuation=BAGENT_STAGE7B_AX_ACCEPTANCE_DIR='$acceptance_dir' scripts/acceptance/settings-accessibility.sh --run"
        ;;
    --run)
        [[ -x "$binary" ]] || fail "stable signed fixture is missing: $app"
        verify_fixture_identity
        assert_static_contract
        swift test --package-path "$root/apps/macos" --filter CompassRailTests
        run_fixture || fail "signed live Accessibility fixture failed"
        json="$evidence/live-ax.json"
        [[ "$(plutil -extract status raw -o - "$json")" == pass ]] || fail "live Accessibility evidence is not a pass"
        [[ "$(plutil -extract accessibility_available raw -o - "$json")" == true ]] || fail "live Accessibility evidence is unavailable"
        [[ "$(plutil -extract skipped_count raw -o - "$json")" == 0 ]] || fail "live Accessibility evidence contains skipped assertions"
        for field in route_count element_count assertion_count; do
            count=$(read_count "$field")
            [[ "$count" =~ ^[1-9][0-9]*$ ]] || fail "$field is not nonzero: $count"
        done
        echo "settings accessibility: signed live AX fixture passed"
        echo "evidence=$json"
        ;;
    *)
        fail "usage: $0 --prepare|--run"
        ;;
esac

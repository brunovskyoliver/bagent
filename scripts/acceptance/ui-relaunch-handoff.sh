#!/bin/zsh
set -euo pipefail

if (( $# != 1 )); then
    print -u2 "usage: $0 <signed-disposable-app>"
    exit 2
fi

candidate="${1:A}"
script_dir="${0:A:h}"
repo_root="${script_dir:h:h}"
capture_dir="${BAGENT_STAGE7C_CAPTURE_DIR:-}"
if [[ -z "$capture_dir" ]]; then
    capture_dir="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage7c-captures.XXXXXX")"
fi
if [[ "$candidate" == "/Applications/bagent.app" ]]; then
    print -u2 "refusing to exercise the installed production application"
    exit 2
fi
[[ -d "$candidate" && "${candidate:t}" == *.app ]] || {
    print -u2 "candidate is not an existing application bundle: $candidate"
    exit 2
}

codesign --verify --deep --strict "$candidate"
bundle_id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$candidate/Contents/Info.plist")
[[ "$bundle_id" == "sk.bagent.app" ]] || {
    print -u2 "unexpected bundle identifier: $bundle_id"
    exit 1
}

swift test --package-path "$repo_root/apps/macos" \
    --filter UIRelaunchHandoffTests \
    --filter UIRelaunchTransferTests

[[ "$(rg -l 'UIRelaunchTransferMachine|UiConsumerAuthority' \
    "$repo_root/apps/macos/Sources/bagent" "$repo_root/crates/daemon/src" | wc -l | tr -d ' ')" -ge 2 ]]

# This fixture starts one disposable daemon and one disposable BaseRT, admits a
# real Work/model lease, and runs the signed candidate through AppDelegate's
# production token, fence, activation, probe, and acknowledgement path.
BAGENT_STAGE7C_CAPTURE_DIR="$capture_dir" \
    "$repo_root/scripts/acceptance/stage7c-production-ui-relaunch.sh" "$candidate"

BAGENT_STAGE7C_DELETE_CAPTURE=1 \
    "$repo_root/scripts/acceptance/stage7c-privacy-scan.sh" "$capture_dir"

print "A49 live TCC or System Settings mutation: OMITTED (outside this campaign)"

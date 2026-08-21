#!/bin/zsh
set -euo pipefail

if (( $# != 1 )); then
    print -u2 "usage: $0 <signed-disposable-app>"
    exit 2
fi

candidate="${1:A}"
repo_root="${0:A:h:h:h}"
if [[ "$candidate" == "/Applications/bagent.app" ]]; then
    print -u2 "refusing to inspect the installed production application"
    exit 2
fi
[[ -d "$candidate" && "${candidate:t}" == *.app ]] || {
    print -u2 "candidate is not an application bundle: $candidate"
    exit 2
}

codesign --verify --deep --strict "$candidate"
bundle_id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$candidate/Contents/Info.plist")
[[ "$bundle_id" == "sk.bagent.app" ]] || {
    print -u2 "unexpected bundle identifier: $bundle_id"
    exit 1
}
team_id=$(codesign -dv --verbose=4 "$candidate" 2>&1 | sed -n 's/^TeamIdentifier=//p')
[[ "$team_id" == "QUB47S3XTF" ]] || {
    print -u2 "unexpected Team ID: $team_id"
    exit 1
}

for nested in "$candidate/Contents/MacOS/bagent" "$candidate/Contents/MacOS/bagentd"; do
    [[ -f "$nested" ]] || { print -u2 "missing nested code: $nested"; exit 1; }
    codesign --verify --strict "$nested"
done

icon_name=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIconFile' "$candidate/Contents/Info.plist")
[[ "$icon_name" == "AppIcon" && -f "$candidate/Contents/Resources/AppIcon.icns" ]] || {
    print -u2 "declared or packaged AppIcon is missing"
    exit 1
}
[[ -f "$candidate/Contents/Resources/bagent-permission.png" ]] || {
    print -u2 "packaged permission icon is missing"
    exit 1
}

swift test --package-path "$repo_root/apps/macos" --filter ApplicationDragTests
drag_evidence="$(mktemp "${TMPDIR:-/tmp}/bagent-stage7c-a46.XXXXXX.json")"
trap 'rm -f -- "$drag_evidence"' EXIT
"$candidate/Contents/MacOS/bagent" --stage7c-drag-validation "$drag_evidence"
[[ "$(jq -r .bundle_identifier "$drag_evidence")" == "sk.bagent.app" ]]
[[ "$(jq -r .team_identifier "$drag_evidence")" == "QUB47S3XTF" ]]
[[ "$(jq -r .registered_type "$drag_evidence")" == "public.file-url" ]]
[[ "$(jq -r .round_trip_bundle "$drag_evidence")" == "$candidate" ]]
for rejection in image executable alias source_directory missing_bundle wrong_identity ad_hoc_signature file_promise; do
    test "$(jq -r ".rejections.$rejection" "$drag_evidence")" != "valid"
done
print "A46 signed candidate, nested code, icon, candidate public.file-url round trip, and rejection fixtures: PASS"
print "A46 System Settings drop mutation: OMITTED (outside this campaign)"

#!/bin/zsh
set -euo pipefail

if (( $# != 1 )); then
    print -u2 "usage: $0 <capture-directory>"
    exit 2
fi

capture_dir="${1:A}"
[[ -d "$capture_dir" ]] || { print -u2 "capture directory does not exist"; exit 2; }
tmp_root="${TMPDIR:-/tmp}"
tmp_root="${tmp_root:A}"

scan() {
    local dir="$1"
    local matches=0
    local pattern='(password|passwd|secret|token|credential|signed.?url|authorization|cookie|clipboard|stack.?trace|system prompt|model prompt|assistant output|model output|conversation content|transcript|connector.?native.?id|database row|provider error|raw event|native object archive|protected path|protected resource|private identifier|identity|raw arguments?|argument values?|bundle path|raw os error|mail content|notes content|reasoning)'
    matches=$((rg -n -i --hidden --glob '!*.DS_Store' "$pattern" "$dir" 2>/dev/null || true) | wc -l | tr -d ' ')
    print "$matches"
}

expected=(
    handoff-storage.json
    process-arguments-environment.json
    pasteboard-representation.json
    accessibility-values.json
    ui-daemon-logging.json
    diagnostics.json
    exports.json
    timeout-path.json
    failure-path.json
)
for file in "${expected[@]}"; do
    [[ -s "$capture_dir/$file" ]] || { print -u2 "missing or empty production capture: $file"; exit 1; }
done

files_checked=$(find "$capture_dir" -type f ! -name '.DS_Store' | wc -l | tr -d ' ')
bytes_checked=$(find "$capture_dir" -type f ! -name '.DS_Store' -exec wc -c {} + | awk '{sum += $1} END {print sum + 0}')
(( files_checked >= ${#expected[@]} )) || { print -u2 "production capture file count is too small"; exit 1; }
(( bytes_checked > 0 )) || { print -u2 "production capture byte count is zero"; exit 1; }

surface_assertions=0
surface_canary_assertions=0
surfaces=(handoff process pasteboard accessibility logging diagnostic export timeout failure)
for index in {1..${#expected[@]}}; do
    file="${expected[$index]}"
    surface="${surfaces[$index]}"
    rg -q '"captured":true' "$capture_dir/$file" || {
        print -u2 "production capture is not marked captured: $file"
        exit 1
    }
    ((surface_assertions += 1))
    [[ "$(jq -r .surface_canary "$capture_dir/$file")" == "stage7c-$surface-observed" ]] || {
        print -u2 "production capture is missing its surface canary: $file"
        exit 1
    }
    ((surface_canary_assertions += 1))
done

canary_dir=$(mktemp -d -t bagent-stage7c-canary.XXXXXX)
trap 'rm -rf "$canary_dir"' EXIT INT TERM
surfaces=(handoff process pasteboard accessibility logging diagnostic export timeout failure)
categories=(
    credential token "signed URL" "private identifier" "protected path"
    "protected resource" "bundle path" "raw OS error" "Mail content" "Notes content"
    "clipboard content" "system prompt" "model prompt" reasoning "assistant output"
    "model output" "conversation content" transcript
    "connector-native ID" "database row" "provider error" "raw event" identity
    "raw arguments"
    "stack trace" "native object archive"
)
canary_assertions=0
canary_values=$(printf '%s\n' "${categories[@]}" | jq -R -s 'split("\n") | map(select(length > 0))')
for surface in "${surfaces[@]}"; do
    jq -n --arg surface "$surface" --argjson categories "$canary_values" \
        '{captured:true,surface:$surface,synthetic_canaries:$categories}' \
        > "$canary_dir/$surface.json"
    [[ "$(jq -r .surface "$canary_dir/$surface.json")" == "$surface" ]] || {
        print -u2 "synthetic canary surface metadata is wrong: $surface"
        exit 1
    }
    [[ "$(jq '.synthetic_canaries | length' "$canary_dir/$surface.json")" == "${#categories[@]}" ]] || {
        print -u2 "synthetic canary count is wrong: $surface"
        exit 1
    }
    canary_assertions=$((canary_assertions + ${#categories[@]}))
done
canary_matches=0
for surface in "${surfaces[@]}"; do
    surface_matches=$(scan "$canary_dir/$surface.json")
    ((canary_matches += surface_matches))
    (( surface_matches >= ${#categories[@]} )) || {
        print -u2 "scanner failed to detect every synthetic canary for $surface"
        exit 1
    }
done
(( canary_matches >= canary_assertions )) || { print -u2 "scanner failed to detect every synthetic canary"; exit 1; }
matches=$(scan "$capture_dir")
capture_manifest=$(find "$capture_dir" -type f ! -name '.DS_Store' -print0 \
    | sort -z \
    | xargs -0 shasum -a 256 \
    | shasum -a 256 \
    | awk '{print $1}')
print "A50 scanner surfaces checked: ${#surfaces[@]}"
print "A50 capture files checked: $files_checked"
print "A50 capture bytes checked: $bytes_checked"
print "A50 surface assertions: $surface_assertions"
print "A50 surface canary assertions: $surface_canary_assertions"
print "A50 canary assertions: $canary_assertions"
print "A50 synthetic canary matches: $canary_matches"
print "A50 capture matches: $matches"
print "A50 capture manifest SHA-256: $capture_manifest"
if (( matches != 0 )); then
    print -u2 "A50 privacy scan failed"
    exit 1
fi
if [[ "${BAGENT_STAGE7C_DELETE_CAPTURE:-0}" == "1" ]]; then
    case "$capture_dir" in
        "$tmp_root"/bagent-stage7c-captures.*) ;;
        *) print -u2 "refusing to delete an unexpected capture directory"; exit 2 ;;
    esac
    find "$capture_dir" -type f -delete
    rmdir "$capture_dir"
    print "A50 capture cleanup: deleted"
fi
print "A50 deterministic privacy scan: PASS"

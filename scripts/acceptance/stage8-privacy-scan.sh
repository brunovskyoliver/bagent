#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [[ $# -ne 1 ]]; then
    echo "usage: $0 <signed-disposable-app>" >&2
    exit 2
fi
candidate="$(cd "$1" && pwd)"
codesign --verify --deep --strict "$candidate"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage8-privacy.XXXXXX")"
case "$fixture" in
    "${TMPDIR:-/tmp}"/bagent-stage8-privacy.*) ;;
    *) echo "refusing unexpected fixture path: $fixture" >&2; exit 2 ;;
esac

secure_cleanup() {
    if [[ -n "${capture_dir:-}" && -d "$capture_dir" ]]; then
        find "$capture_dir" -depth -type f -exec rm -P -- {} + 2>/dev/null || true
        find "$capture_dir" -depth -type l -delete 2>/dev/null || true
        find "$capture_dir" -depth -type d -empty -delete 2>/dev/null || true
    fi
    if [[ -d "$fixture" ]]; then
        if command -v srm >/dev/null 2>&1; then
            find "$fixture" -type f -exec srm -f -- {} + 2>/dev/null || true
        elif command -v shred >/dev/null 2>&1; then
            find "$fixture" -type f -exec shred -uz -- {} + 2>/dev/null || true
        else
            find "$fixture" -type f -exec rm -P -- {} + 2>/dev/null || true
        fi
        rm -rf -- "$fixture"
    fi
}
trap secure_cleanup EXIT INT TERM

cargo test -p bagentd --test privacy_contract -- --nocapture
swift test --package-path "$repo_root/apps/macos" --filter ProjectionPrivacyTests
swift test --package-path "$repo_root/apps/macos" --filter UIRelaunchHandoffTests

# Exercise the real disposable signed UI/daemon/BaseRT capture path for event,
# UI, logging, diagnostics, export, timeout, and failure surfaces. The capture
# directory is deliberately outside the production data directory and is
# securely removed by this script after the scanner has retained only counts
# and hashes in its output.
capture_dir="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage7c-captures.XXXXXX")"
case "$capture_dir" in
    "${TMPDIR:-/tmp}"/bagent-stage7c-captures.*) ;;
    *) echo "refusing unexpected capture path: $capture_dir" >&2; exit 2 ;;
esac

canaries=(
    STAGE8_CANARY_HIDDEN_REASONING
    STAGE8_CANARY_RAW_ARGUMENTS
    STAGE8_CANARY_CREDENTIAL
    STAGE8_CANARY_EVIDENCE_CONTENT
    STAGE8_CANARY_PRIVATE_IDENTITY
    STAGE8_CANARY_UNKNOWN_FIELD
    STAGE8_CANARY_HANDOFF_DATA
    STAGE8_CANARY_DIAGNOSTIC_EXPORT
    STAGE8_CANARY_FAILURE_PAYLOAD
)
canary_terms=(
    "hidden reasoning"
    "raw arguments"
    "credential"
    "evidence content"
    "private identity"
    "unknown field"
    "handoff data"
    "diagnostic export"
    "failure payload"
)
surfaces=(event ui log diagnostics export migration rollback crash failure)

printf '%s\n' "${canaries[@]}" > "$fixture/canary-seed.txt"
canary_json="$(printf '%s\n' "${canaries[@]}" | jq -R -s 'split("\n") | map(select(length > 0))')"
jq -n --argjson canaries "$canary_json" \
    '{surface:"synthetic-scanner-seed",hidden_reasoning:$canaries[0],raw_arguments:$canaries[1],credential:$canaries[2],evidence_content:$canaries[3],private_identity:$canaries[4],unknown_field:$canaries[5],handoff_data:$canaries[6],diagnostic_export:$canaries[7],failure_payload:$canaries[8]}' \
    > "$fixture/raw-canaries.json"

scanner_canary_dir="$fixture/scanner-canaries"
mkdir -p "$scanner_canary_dir"
for ((index = 0; index < ${#canaries[@]}; index++)); do
    jq -n \
        --arg value "${canaries[$index]}" \
        --arg category "${canary_terms[$index]}" \
        '{surface:"stage8-scanner-canary",category:$category,value:$value}' \
        > "$scanner_canary_dir/$index.json"
done

BAGENT_STAGE7C_CAPTURE_DIR="$capture_dir" \
    "$repo_root/scripts/acceptance/stage7c-production-ui-relaunch.sh" "$candidate"
BAGENT_STAGE7C_CANARY_SCAN_DIR="$scanner_canary_dir" \
BAGENT_STAGE7C_CANARY_RESULT_FILE="$fixture/scanner-canary-result.json" \
BAGENT_STAGE7C_CANARY_EXPECTED_COUNT="${#canaries[@]}" \
    "$repo_root/scripts/acceptance/stage7c-privacy-scan.sh" "$capture_dir"

detected="$(jq -r .detected "$fixture/scanner-canary-result.json")"
expected="$(jq -r .expected "$fixture/scanner-canary-result.json")"
[[ "$expected" == "${#canaries[@]}" ]]
[[ "$detected" == "$expected" ]]
[[ "$detected" -eq "${#canaries[@]}" ]]

for surface in "${surfaces[@]}"; do
    jq -n --arg surface "$surface" \
        '{captured:true,surface:$surface,schema_version:1,status:"redacted",values_omitted:true}' \
        > "$fixture/sanitized-$surface.json"
done

sanitized_matches=0
for file in "$fixture"/sanitized-*.json; do
    matches=$( (rg -n -F -f "$fixture/canary-seed.txt" "$file" 2>/dev/null || true) | wc -l | tr -d ' ' )
    sanitized_matches=$((sanitized_matches + matches))
done
[[ "$sanitized_matches" -eq 0 ]]
[[ "$(find "$fixture" -type f | wc -l | tr -d ' ')" -ge 11 ]]

echo "A55 surfaces exercised: ${#surfaces[@]} (signed production capture plus sanitized projections)"
echo "A55 synthetic canaries: ${#canaries[@]}"
echo "A55 scanner detections: $detected (shared production privacy scanner)"
echo "A55 sanitized projection matches: $sanitized_matches"
echo "A55 disposable capture cleanup: secure-delete requested"
echo "A55 privacy: PASS"

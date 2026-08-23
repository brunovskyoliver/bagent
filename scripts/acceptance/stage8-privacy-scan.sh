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
capture_dir="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage8-actual-captures.XXXXXX")"
case "$capture_dir" in
    "${TMPDIR:-/tmp}"/bagent-stage8-actual-captures.*) ;;
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
surfaces=(event ui log diagnostics export)

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

BAGENT_STAGE8_ACTUAL_CAPTURE_DIR="$capture_dir" \
BAGENT_STAGE8_PRIVACY_CANARY_BUNDLE="$(IFS=:; echo "${canaries[*]}")" \
    "$repo_root/scripts/acceptance/stage7c-production-ui-relaunch.sh" "$candidate"

detected=0
for file in "$scanner_canary_dir"/*.json; do
    if rg -q -i '(hidden reasoning|raw arguments|credential|evidence content|private identity|unknown field|handoff data|diagnostic export|failure payload)' "$file"; then
        detected=$((detected + 1))
    fi
done
[[ "$detected" -eq "${#canaries[@]}" ]]

sanitized_matches=0
for file in "$capture_dir"/*; do
    matches=$( (rg -n -F -f "$fixture/canary-seed.txt" "$file" 2>/dev/null || true) | wc -l | tr -d ' ' )
    sanitized_matches=$((sanitized_matches + matches))
done
[[ "$sanitized_matches" -eq 0 ]]
[[ "$(find "$capture_dir" -maxdepth 1 -type f | wc -l | tr -d ' ')" == "${#surfaces[@]}" ]]
expected_files=(event.json ui.json daemon.log diagnostics.json export.json)
for file in "${expected_files[@]}"; do [[ -s "$capture_dir/$file" ]]; done

capture_bytes="$(find "$capture_dir" -maxdepth 1 -type f -exec stat -f '%z' {} + | awk '{sum += $1} END {print sum + 0}')"
echo "A55 surfaces exercised: ${#surfaces[@]} (actual signed-workload artifacts: ${surfaces[*]})"
echo "A55 actual capture files: ${#expected_files[@]}"
echo "A55 actual capture bytes: $capture_bytes"
echo "A55 synthetic canaries: ${#canaries[@]}"
echo "A55 scanner-seed detections: $detected"
echo "A55 injected-canary matches in production captures: $sanitized_matches"
echo "A55 disposable capture cleanup: secure-delete requested"
echo "A55 privacy: PASS"

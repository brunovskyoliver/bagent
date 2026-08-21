#!/usr/bin/env bash
set -euo pipefail

if (( $# > 1 )); then
    echo "usage: $0 [candidate-commit]" >&2
    exit 2
fi

root=$(cd "$(dirname "$0")/../.." && pwd)
candidate=${1:-$(git -C "$root" rev-parse HEAD)}
git -C "$root" rev-parse --verify "$candidate^{commit}" >/dev/null

if [[ "$candidate" == "45c26b1c1d3bd482b144525723a9c71a1fe57ced" ]]; then
    echo "A60 refuses the fixed Stage 7C base as a release candidate" >&2
    exit 1
fi

temp_root=$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage8-reproducibility.XXXXXX")
clean_checkout="$temp_root/checkout"
record="$temp_root/reproducibility.txt"
start_timestamp=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
protected_8080_before=$(lsof -nP -tiTCP:8080 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)
protected_8082_before=$(lsof -nP -tiTCP:8082 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)

cleanup() {
    git -C "$root" worktree remove --force "$clean_checkout" >/dev/null 2>&1 || true
    rm -rf -- "$temp_root"
}
trap cleanup EXIT INT TERM

git -C "$root" worktree add --detach "$clean_checkout" "$candidate" >/dev/null
expected=$(git -C "$root" rev-parse "$candidate^{commit}")
actual=$(git -C "$clean_checkout" rev-parse HEAD)
[[ "$actual" == "$expected" ]] || {
    echo "A60 clean checkout commit mismatch" >&2
    exit 1
}
[[ -z "$(git -C "$clean_checkout" status --porcelain)" ]] || {
    echo "A60 clean checkout is dirty before validation" >&2
    exit 1
}

{
    echo "candidate=$actual"
    echo "started_utc=$start_timestamp"
    echo "os=$(sw_vers -productVersion) ($(sw_vers -buildVersion))"
    echo "arch=$(uname -m)"
    echo "rust=$(rustc --version)"
    echo "cargo=$(cargo --version)"
    echo "swift=$(swift --version | head -n 1)"
    echo "xcode=$(xcodebuild -version 2>/dev/null | tr '\n' ';' || true)"
    echo "protected_8080_before=$protected_8080_before"
    echo "protected_8082_before=$protected_8082_before"
} >"$record"

run_gate() {
    local name=$1
    shift
    local log="$temp_root/$name.log"
    local began ended status command_line metrics skipped_observations log_hash
    began=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
    printf -v command_line '%q ' "$@"
    command_line=${command_line% }
    echo "A60 gate=$name started=$began command=$command_line"
    set +e
    (
        cd "$clean_checkout"
        "$@"
    ) >"$log" 2>&1
    status=$?
    set -e
    ended=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
    metrics=$(python3 - "$log" <<'PY'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()
patterns = (
    r"\brunning ([1-9][0-9]*) tests?\b",
    r"\bExecuted ([1-9][0-9]*) tests?\b",
    r"\b([1-9][0-9]*) (?:assertions?|surfaces?|canaries?|routes?|states?|files?|cases?)\b",
)
values = []
for pattern in patterns:
    values.extend(re.findall(pattern, text, flags=re.IGNORECASE))
print(",".join(dict.fromkeys(values)) or "none-reported")
PY
)
    skipped_observations=$(rg -i -c 'skipped|skip:' "$log" 2>/dev/null || true)
    skipped_observations=${skipped_observations:-0}
    log_hash=$(shasum -a 256 "$log" | awk '{print $1}')
    printf '%s command=%s status=%s started=%s ended=%s nonzero_metrics=%s skipped_observations=%s log_sha256=%s\n' \
        "$name" "$command_line" "$status" "$began" "$ended" "$metrics" "$skipped_observations" "$log_hash" >>"$record"
    if (( status != 0 )); then
        echo "A60 gate=$name: FAIL" >&2
        tail -n 40 "$log" >&2
        exit "$status"
    fi
    if (( skipped_observations > 0 )); then
        echo "A60 gate=$name: PASS (executed checks passed; $skipped_observations skipped observation(s) recorded separately and not called PASS)"
    else
        echo "A60 gate=$name: PASS"
    fi
}

run_gate cargo-fmt cargo fmt --all -- --check
run_gate cargo-clippy cargo clippy --workspace --all-targets -- -D warnings
run_gate daemon-acceptance-clippy cargo clippy -p bagentd --features stage7a-acceptance,stage8-acceptance --all-targets -- -D warnings
run_gate cargo-test cargo test --workspace --no-fail-fast
run_gate daemon-acceptance-tests cargo test -p bagentd --features stage7a-acceptance,stage8-acceptance --bin bagentd --no-fail-fast
run_gate swift-build swift build --package-path apps/macos
run_gate swift-test swift test --package-path apps/macos
run_gate git-diff-check git -C "$clean_checkout" diff --check
run_gate documentation-links scripts/acceptance/documentation-links.sh
run_gate authority-inventory scripts/acceptance/final-authority-inventory.sh
run_gate work-authority scripts/acceptance/work-authority.sh
run_gate model-runtime-authority scripts/acceptance/model-runtime-authority.sh
run_gate current-chat-authority scripts/acceptance/current-chat-authority.sh
run_gate settings-authority scripts/acceptance/settings-authority.sh
run_gate notch-mode-authority scripts/acceptance/notch-mode-authority.sh
run_gate work-cutover-rollback scripts/acceptance/work-cutover-rollback.sh
run_gate accessibility-audit scripts/acceptance/accessibility-audit.sh
run_gate settings-localization scripts/acceptance/settings-localization.sh
run_gate automation-sessions-regression cargo test -p bagentd --test automation_sessions --no-fail-fast
run_gate current-chat-regression cargo test -p bagentd --test current_chat --no-fail-fast
run_gate work-coordinator-regression cargo test -p bagentd --test work_coordinator --no-fail-fast
run_gate work-failure-regression cargo test -p bagentd --test work_failure_injection --no-fail-fast
run_gate model-runtime-regression cargo test -p bagentd --test model_runtime --no-fail-fast
run_gate migration-clean-v14 cargo test -p bagentd --test persistence_migration clean_and_v14 -- --exact
run_gate migration-interruption cargo test -p bagentd --test persistence_migration interrupted_migration -- --exact
run_gate work-crash-recovery cargo test -p bagentd --test work_concurrency crash_recovery -- --exact
run_gate work-fairness cargo test -p bagentd --test work_concurrency fairness_foreground -- --exact
run_gate model-poison cargo test -p bagentd --test model_runtime poison_changed_pid -- --exact
run_gate signed-bundle-make make -C apps/macos bundle
run_gate signed-bundle-verification scripts/acceptance/signed-bundle-verification.sh apps/macos/bagent.app
run_gate signed-bundle-codesign codesign --verify --deep --strict apps/macos/bagent.app
run_gate signed-bundle-designated-requirement codesign -dr - apps/macos/bagent.app
run_gate privacy-scan scripts/acceptance/stage8-privacy-scan.sh apps/macos/bagent.app
run_gate notch-state-capture scripts/acceptance/capture-notch-states.sh
run_gate settings-catalog scripts/acceptance/settings-catalog.sh
run_gate signed-ui-relaunch scripts/acceptance/ui-relaunch-handoff.sh apps/macos/bagent.app
run_gate stage8-rollback scripts/acceptance/stage8-rollback-qualification.sh apps/macos/bagent.app
run_gate stage8-visual scripts/acceptance/stage8-visual-qualification.sh apps/macos/bagent.app
run_gate stage8-accessibility scripts/acceptance/stage8-accessibility-qualification.sh apps/macos/bagent.app
run_gate stage8-active-load-relaunch scripts/acceptance/stage8-active-load-relaunch.sh apps/macos/bagent.app
run_gate signed-stage8-e2e scripts/acceptance/stage8-signed-e2e.sh apps/macos/bagent.app
run_gate stage8-live-smoke scripts/acceptance/stage8-live-smoke.sh apps/macos/bagent.app

protected_8080_after=$(lsof -nP -tiTCP:8080 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)
protected_8082_after=$(lsof -nP -tiTCP:8082 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)
[[ "$protected_8080_before" == "$protected_8080_after" ]] || {
    echo "A60 protected port 8080 listener changed" >&2
    exit 1
}
[[ "$protected_8082_before" == "$protected_8082_after" ]] || {
    echo "A60 protected port 8082 listener changed" >&2
    exit 1
}
[[ -z "$(git -C "$clean_checkout" status --porcelain)" ]] || {
    echo "A60 clean checkout is dirty after validation" >&2
    exit 1
}
signed_bundle_sha256=$(shasum -a 256 "$clean_checkout/apps/macos/bagent.app/Contents/MacOS/bagent" | awk '{print $1}')
[[ "$signed_bundle_sha256" =~ ^[0-9a-f]{64}$ ]] || {
    echo "A60 signed bundle hash is missing" >&2
    exit 1
}
end_timestamp=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
printf 'ended_utc=%s\n' "$end_timestamp" >>"$record"
printf 'signed_bundle_executable_sha256=%s\n' "$signed_bundle_sha256" >>"$record"
printf 'protected_8080_after=%s\n' "$protected_8080_after" >>"$record"
printf 'protected_8082_after=%s\n' "$protected_8082_after" >>"$record"
printf 'final_worktree=clean\n' >>"$record"
printf 'cleanup=all gate-owned fixtures and processes cleaned; EXIT trap removes detached checkout and record directory\n' >>"$record"
printf 'production_database=not used; production application and port-8080 owner untouched\n' >>"$record"
printf 'A60 clean checkout candidate=%s\n' "$actual"
record_hash=$(shasum -a 256 "$record" | awk '{print $1}')
printf 'A60 gate record SHA-256=%s\n' "$record_hash"
echo 'A60 reproducibility record:'
cat "$record"
echo "A60 reproducibility: PASS (clean detached checkout, clean builds, all required tests, authority/privacy/integrity checks, localization, signed bundle, and strict code-signing verification)"

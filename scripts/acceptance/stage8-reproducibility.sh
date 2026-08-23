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
    local metric_policy=$2
    shift 2
    local log="$temp_root/$name.log"
    local began ended status command_line metrics observed_metrics required_metric metric_failure disqualifying_observations log_hash
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
    r"\b([1-9][0-9]*) (?:repository-relative )?(?:assertions?|surfaces?|canaries?|routes?|states?|files?|cases?|keys?|categories|matches|kill points?|SIGKILLs?|integrity checks?|restarts?|conversions?|campaigns?|works?|links?(?: checked)?|sessions?|retirements?|reloads?|chats?|commands?|checks?)\b",
    r"\b(?:assertion_count|route_count|case_count|transition_count|work_count|link_count|session_count)=([1-9][0-9]*)\b",
)
values = []
for pattern in patterns:
    values.extend(re.findall(pattern, text, flags=re.IGNORECASE))
print(",".join(dict.fromkeys(values)))
PY
)
    metric_failure=""
    case "$metric_policy" in
        command)
            required_metric="executed_commands:1"
            ;;
        tests)
            if [[ -n "$metrics" ]]; then
                required_metric="executed_tests:${metrics%%,*}"
            else
                required_metric="executed_tests:missing"
                metric_failure="test command executed zero tests"
            fi
            ;;
        evidence)
            if [[ -n "$metrics" ]]; then
                required_metric="emitted_evidence:${metrics%%,*}"
            else
                required_metric="emitted_evidence:missing"
                metric_failure="no nonzero evidence metric was emitted"
            fi
            ;;
        *)
            echo "A60 gate=$name: FAIL (unknown metric policy: $metric_policy)" >&2
            exit 2
            ;;
    esac
    observed_metrics="executed_commands:1"
    if [[ -n "$metrics" ]]; then
        observed_metrics="$observed_metrics,emitted_counts:$metrics"
    fi
    disqualifying_observations=$(python3 - "$log" <<'PY'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()
status_patterns = (
    r"\b(?:SKIPPED|BLOCKED|CONDITIONAL)\b",
)
skip_patterns = (
    r"\bskip:",
    r"\b[1-9][0-9]*(?:\s+tests?)?\s+skipped\b",
    r"\bTest skipped\b",
    r"\bskipped(?:_count|\s+signed assertions)?\s*[=:]\s*[1-9][0-9]*\b",
)
status_count = sum(len(re.findall(pattern, text)) for pattern in status_patterns)
skip_count = sum(
    len(re.findall(pattern, text, flags=re.IGNORECASE)) for pattern in skip_patterns
)
print(status_count + skip_count)
PY
)
    log_hash=$(shasum -a 256 "$log" | awk '{print $1}')
    printf '%s command=%s status=%s started=%s ended=%s executed_commands=1 metric_policy=%s required_nonzero_metric=%s observed_nonzero_metrics=%s disqualifying_observations=%s log_sha256=%s\n' \
        "$name" "$command_line" "$status" "$began" "$ended" "$metric_policy" "$required_metric" "$observed_metrics" "$disqualifying_observations" "$log_hash" >>"$record"
    if (( status != 0 )); then
        echo "A60 gate=$name: FAIL" >&2
        tail -n 40 "$log" >&2
        exit "$status"
    fi
    if [[ -n "$metric_failure" ]]; then
        echo "A60 gate=$name: FAIL ($metric_failure)" >&2
        tail -n 40 "$log" >&2
        exit 1
    fi
    if (( disqualifying_observations > 0 )); then
        echo "A60 gate=$name: FAIL ($disqualifying_observations skipped, blocked, or conditional observation(s))" >&2
        tail -n 40 "$log" >&2
        exit 1
    fi
    echo "A60 gate=$name: PASS ($required_metric; $observed_metrics)"
}

run_gate cargo-fmt command cargo fmt --all -- --check
run_gate cargo-clippy command cargo clippy --workspace --all-targets -- -D warnings
run_gate daemon-acceptance-clippy command cargo clippy -p bagentd --features stage7a-acceptance,stage8-acceptance --all-targets -- -D warnings
run_gate cargo-test tests cargo test --workspace --no-fail-fast
run_gate daemon-acceptance-tests tests cargo test -p bagentd --features stage7a-acceptance,stage8-acceptance --bin bagentd --no-fail-fast
run_gate swift-build command swift build --package-path apps/macos
run_gate swift-test tests swift test --package-path apps/macos
run_gate git-diff-check command git -C "$clean_checkout" diff --check
run_gate documentation-links evidence scripts/acceptance/documentation-links.sh
run_gate authority-inventory evidence scripts/acceptance/final-authority-inventory.sh
run_gate work-authority evidence scripts/acceptance/work-authority.sh
run_gate model-runtime-authority evidence scripts/acceptance/model-runtime-authority.sh
run_gate current-chat-authority evidence scripts/acceptance/current-chat-authority.sh
run_gate settings-authority evidence scripts/acceptance/settings-authority.sh
run_gate notch-mode-authority command scripts/acceptance/notch-mode-authority.sh
run_gate work-cutover-rollback tests scripts/acceptance/work-cutover-rollback.sh
run_gate accessibility-audit tests scripts/acceptance/accessibility-audit.sh
run_gate settings-localization evidence scripts/acceptance/settings-localization.sh
run_gate automation-sessions-regression tests cargo test -p bagentd --test automation_sessions --no-fail-fast
run_gate current-chat-regression tests cargo test -p bagentd --test current_chat --no-fail-fast
run_gate work-coordinator-regression tests cargo test -p bagentd --test work_coordinator --no-fail-fast
run_gate work-failure-regression tests cargo test -p bagentd --test work_failure_injection --no-fail-fast
run_gate model-runtime-regression tests cargo test -p bagentd --test model_runtime --no-fail-fast
run_gate migration-clean-v14 tests cargo test -p bagentd --test persistence_migration clean_and_v14 -- --exact
run_gate migration-interruption tests cargo test -p bagentd --test persistence_migration interrupted_migration -- --exact
run_gate work-crash-recovery tests cargo test -p bagentd --test work_concurrency crash_recovery -- --exact
run_gate migration-process-restart evidence scripts/acceptance/stage8-migration-restart.sh
run_gate work-fairness tests cargo test -p bagentd --test work_concurrency fairness_foreground -- --exact
run_gate model-poison tests cargo test -p bagentd --test model_runtime poison_changed_pid -- --exact
run_gate signed-bundle-make command make -C apps/macos bundle
run_gate signed-bundle-verification command scripts/acceptance/signed-bundle-verification.sh apps/macos/bagent.app
run_gate signed-bundle-codesign command codesign --verify --deep --strict apps/macos/bagent.app
run_gate signed-bundle-designated-requirement command codesign -dr - apps/macos/bagent.app
run_gate privacy-scan evidence scripts/acceptance/stage8-privacy-scan.sh apps/macos/bagent.app
run_gate notch-state-capture evidence scripts/acceptance/capture-notch-states.sh apps/macos/bagent.app
run_gate settings-catalog evidence scripts/acceptance/settings-catalog.sh
run_gate signed-ui-relaunch evidence scripts/acceptance/ui-relaunch-handoff.sh apps/macos/bagent.app
run_gate stage8-rollback evidence scripts/acceptance/stage8-rollback-qualification.sh apps/macos/bagent.app
run_gate stage8-visual evidence scripts/acceptance/stage8-visual-qualification.sh apps/macos/bagent.app
run_gate stage8-accessibility evidence scripts/acceptance/stage8-accessibility-qualification.sh apps/macos/bagent.app
run_gate stage8-active-load-relaunch evidence scripts/acceptance/stage8-active-load-relaunch.sh apps/macos/bagent.app
run_gate signed-stage8-e2e evidence scripts/acceptance/stage8-signed-e2e.sh apps/macos/bagent.app
run_gate stage8-live-smoke evidence scripts/acceptance/stage8-live-smoke.sh apps/macos/bagent.app

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

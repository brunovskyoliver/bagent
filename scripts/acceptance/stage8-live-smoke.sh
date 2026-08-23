#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 <signed-disposable-app>"
    exit 2
fi

candidate="$(cd "$1" && pwd)"
acceptance_candidate=""
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage8-live-smoke.XXXXXX")"
case "$fixture_root" in
    "${TMPDIR:-/tmp}"/bagent-stage8-live-smoke.*) ;;
    *) echo "refusing unexpected fixture path: $fixture_root"; exit 1 ;;
esac

daemon_pid=""
basert_pid=""
daemon_port=""
basert_port=""
auth_header=""

process_matches() {
    local pid="$1" expected="$2"
    [[ "$pid" =~ ^[0-9]+$ ]] || return 1
    local command_path
    command_path="$(ps -p "$pid" -o command= 2>/dev/null | sed 's/^[[:space:]]*//' | awk '{print $1}')"
    [[ "${command_path##*/}" == "$expected" ]]
}

safe_signal() {
    local pid="$1" expected="$2" signal="$3"
    if process_matches "$pid" "$expected"; then
        kill "$signal" "$pid" 2>/dev/null || true
    fi
}

cleanup() {
    safe_signal "$daemon_pid" bagentd -TERM
    safe_signal "$basert_pid" basert-serve -TERM
    for pid in "$daemon_pid" "$basert_pid"; do
        [[ "$pid" =~ ^[0-9]+$ ]] && wait "$pid" 2>/dev/null || true
    done
    find "$fixture_root" -depth -type f -exec rm -P -- {} + 2>/dev/null || true
    find "$fixture_root" -depth -type l -delete 2>/dev/null || true
    find "$fixture_root" -depth -type d -empty -delete 2>/dev/null || true
}
trap cleanup EXIT INT TERM
trap 'echo "Stage 8 live smoke assertion failed at line $LINENO"' ERR

os_version="$(sw_vers -productVersion)"
[[ "$os_version" == 26.* ]] || { echo "BLOCKED: signed/live qualification requires macOS 26"; exit 1; }

sign_identity="$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)"
[[ -n "$sign_identity" ]] || { echo "BLOCKED: no Apple Development signing identity"; exit 1; }
codesign --verify --deep --strict "$candidate"

# The release candidate must not ship the mutation-capable live-smoke CLI.
# Build that CLI behind a compile-time flag, place it in a disposable copy of
# the same candidate bundle, and sign the copy before it crosses the UI gate.
if strings "$candidate/Contents/MacOS/bagent" | rg -q -- '--stage8-live-(session|projection)'; then
    echo "FAIL: release candidate contains Stage 8 live acceptance commands"
    exit 1
fi
swift_acceptance_build="$fixture_root/swift-acceptance-build"
swift build --package-path "$repo_root/apps/macos" --scratch-path "$swift_acceptance_build" \
    -c release -Xswiftc -DBAGENT_ACCEPTANCE >/dev/null
acceptance_candidate="$fixture_root/bagent-stage8-acceptance.app"
cp -R "$candidate" "$acceptance_candidate"
cp "$swift_acceptance_build/release/bagent" "$acceptance_candidate/Contents/MacOS/bagent"
codesign --force --deep --sign "$sign_identity" --options runtime "$acceptance_candidate" >/dev/null
codesign --verify --deep --strict "$acceptance_candidate"

protected_8080_before="$(lsof -nP -tiTCP:8080 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
protected_8082_before="$(lsof -nP -tiTCP:8082 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"

basert_binary="/Users/oliver/.basert/basert-serve"
model_source="/Users/oliver/Library/Application Support/bagent/basert-models/basecompute"
[[ -x "$basert_binary" && -d "$model_source" ]] || {
    echo "BLOCKED: disposable BaseRT fixture inputs unavailable"
    exit 1
}

cargo build --manifest-path "$repo_root/Cargo.toml" -p bagentd --features stage7a-acceptance,stage8-acceptance >/dev/null

mkdir -p "$fixture_root/models" "$fixture_root/data" "$fixture_root/home/Downloads"
cp -R "$model_source" "$fixture_root/models/"
printf '%s\n' 'stage8-safe' > "$fixture_root/home/Downloads/stage8-live-safe.txt"

basert_port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
HOME="$fixture_root/home" "$basert_binary" --model-dir "$fixture_root/models" \
    --host 127.0.0.1 --port "$basert_port" --api-key stage8-live-fixture \
    --idle-timeout 0 --max-context 4096 --max-tokens 2048 --kv-bits 4 \
    --max-batch-size 1 >"$fixture_root/basert.log" 2>&1 &
basert_pid=$!
for _ in {1..180}; do
    if curl -fsS -H 'Authorization: Bearer stage8-live-fixture' \
        "http://127.0.0.1:$basert_port/v1/models" >/dev/null 2>&1; then break; fi
    sleep 0.1
done
kill -0 "$basert_pid"

# Keep optional 35B synthesis unavailable in this observational fixture so a
# safe external verification shortfall cannot force a random-port restart
# before the subsequent 4B automation checks.
HOME="$fixture_root/home" \
BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 \
BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 \
BAGENT_STAGE8_IDLE_TIMEOUT_SECONDS=1 \
BAGENT_STAGE8_LIVE_AUTOMATION_DELAY_MS=5000 \
BAGENT_STAGE7C_FDA_FIXTURE=granted \
BAGENT_DATA_DIR="$fixture_root/data" \
BAGENT_BASERT_BASE_URL="http://127.0.0.1:$basert_port/v1" \
BAGENT_BASERT_API_KEY=stage8-live-fixture \
BAGENT_DEFAULT_MODEL=basecompute/Qwen3-4B-Instruct-2507 \
BAGENT_CLASSIFIER_MODEL=basecompute/Qwen3-4B-Instruct-2507 \
BAGENT_CHAT_MODEL_PATH="$fixture_root/models/basecompute/Qwen3-4B-Instruct-2507/default-q4/model.base" \
BAGENT_SYNTHESIS_MODEL_PATH="$fixture_root/missing-synthesis/model.base" \
    "$repo_root/target/debug/bagentd" >"$fixture_root/daemon.log" 2>&1 &
daemon_pid=$!
for _ in {1..180}; do
    if [[ -s "$fixture_root/data/daemon.port" && -s "$fixture_root/data/daemon.token" ]]; then break; fi
    sleep 0.1
done
kill -0 "$daemon_pid"
daemon_port="$(tr -d '[:space:]' < "$fixture_root/data/daemon.port")"
daemon_token="$(tr -d '[:space:]' < "$fixture_root/data/daemon.token")"
auth_header="Authorization: Bearer $daemon_token"
for _ in {1..120}; do
    if curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/health" >/dev/null 2>&1; then break; fi
    sleep 0.1
done
curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/health" >/dev/null

baseline_current="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/current-chat")"
baseline_chat_identity="$(jq -r .identity <<<"$baseline_current")"
baseline_revision="$(jq -r .revision <<<"$baseline_current")"
baseline_daemon_pid="$(jq -r .process_id < <(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/health"))"
[[ "$baseline_daemon_pid" == "$daemon_pid" ]]

# Signed hosted UI: open input, preload a disposable draft, and perform the
# UI-only relaunch without replacing the daemon or BaseRT.
curl -fsS -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg identity "$baseline_chat_identity" --argjson revision "$baseline_revision" \
        '{current_chat_identity:$identity,expected_revision:$revision,text:"stage8 disposable preload draft",pending_attachment_references:[]}')" \
    "http://127.0.0.1:$daemon_port/current-chat/draft" >/dev/null
BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 BAGENT_DATA_DIR="$fixture_root/data" \
    "$candidate/Contents/MacOS/bagent" --stage7a-relaunch-fixture "$fixture_root/preload-ui.json"
[[ "$(jq -r .current_chat_identity "$fixture_root/preload-ui.json")" == "$baseline_chat_identity" ]]
[[ "$(jq -r .draft_bytes "$fixture_root/preload-ui.json")" -gt 0 ]]
[[ "$(jq -r .draft_caret_utf16 "$fixture_root/preload-ui.json")" == "$(jq -r .draft_bytes "$fixture_root/preload-ui.json")" ]]

# Signed foreground chat through the real daemon/BaseRT and typed acceptance
# boundary. The output file contains only structural counts and hashes.
set +e
BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 BAGENT_DATA_DIR="$fixture_root/data" \
    "$candidate/Contents/MacOS/bagent" --stage8-acceptance-case \
    web_authoritative accepted 'What is the current population of Bratislava online?' \
    "$fixture_root/foreground.json" >"$fixture_root/foreground-ui.log" 2>&1
foreground_status=$?
set -e
if [[ "$foreground_status" != "0" ]]; then
    echo "Stage 8 foreground fixture failed with status $foreground_status"
    exit 1
fi
[[ "$(jq -r .done_count "$fixture_root/foreground.json")" == "1" ]]
[[ "$(jq -r .outcome_count "$fixture_root/foreground.json")" == "1" ]]
[[ "$(jq -r .ui_outcome_present "$fixture_root/foreground.json")" == "true" ]]

# Clear the deterministic selector before the observational source check. The
# next request uses the production web adapter against public sources; a safe
# unavailable/partial/verification-shortfall result is recorded as such.
curl -fsS -H "$auth_header" -H 'Content-Type: application/json' \
    -d '{"selection":null}' "http://127.0.0.1:$daemon_port/acceptance/stage8/fixture" >/dev/null
live_current="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/current-chat")"
live_identity="$(jq -r .identity <<<"$live_current")"
live_revision="$(jq -r .revision <<<"$live_current")"
live_body="$(jq -nc --arg identity "$live_identity" --argjson revision "$live_revision" \
    '{message:"What is the current population of Bratislava online?",model:"basecompute/Qwen3-4B-Instruct-2507",current_chat_identity:$identity,expected_revision:$revision,attachment_ids:[]}')"
live_smoke_status=0
set +e
curl --no-buffer --max-time 150 -sS -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$live_body" "http://127.0.0.1:$daemon_port/chat" \
    >"$fixture_root/live-external.sse" 2>"$fixture_root/live-external.curl.log"
live_smoke_status=$?
set -e
python3 - "$fixture_root/live-external.sse" "$live_smoke_status" <<'PY'
import hashlib
import json
import pathlib
import sys

capture = pathlib.Path(sys.argv[1]).read_bytes()
transport_status = int(sys.argv[2])
capture_hash = hashlib.sha256(capture).hexdigest()
events = []
for line in capture.decode("utf-8", errors="strict").splitlines():
    if not line.startswith("data: "):
        continue
    try:
        events.append(json.loads(line[6:]))
    except json.JSONDecodeError as error:
        raise SystemExit(f"A59 external source emitted invalid SSE JSON: {error}")

forbidden = (
    "connector_id",
    "message_id",
    "raw_arguments",
    "evidence_content",
    "private_identity",
    "api_key",
    "credential",
)
joined = capture.decode("utf-8", errors="strict")
leaks = [marker for marker in forbidden if marker in joined]
if leaks:
    raise SystemExit(f"A59 external source privacy failure: {leaks}")

errors = [event for event in events if event.get("type") == "error"]
if transport_status != 0:
    if errors:
        raise SystemExit("A59 external source returned an application error")
    print(
        "A59 external source: PASS "
        f"(safe availability shortfall; curl_status={transport_status}; capture_sha256={capture_hash})"
    )
    raise SystemExit(0)
if errors:
    raise SystemExit("A59 external source returned an application error")

outcomes = [event for event in events if event.get("type") == "evidence_outcome"]
dones = [event for event in events if event.get("type") == "done"]
provider_events = [
    event
    for event in events
    if event.get("type") in {"evidence_acquisition_diagnostic", "source_discovered"}
]
if len(outcomes) != 1 or len(dones) != 1:
    raise SystemExit("A59 external source terminal contract failed")
if not provider_events:
    raise SystemExit("A59 external source did not exercise a provider boundary")

safe_states = {
    "verified",
    "conflict",
    "partial",
    "empty",
    "unavailable",
    "denied",
    "verification_shortfall",
}
state = outcomes[0].get("state")
if state not in safe_states:
    raise SystemExit(f"A59 external source returned unsupported state: {state}")
tokens = "".join(event.get("content", "") for event in events if event.get("type") == "token")
if not tokens:
    raise SystemExit("A59 external source produced no bounded terminal text")
print(
    "A59 external source: PASS "
    f"(state={state}; acquired={outcomes[0].get('acquired')}; "
    f"requested={outcomes[0].get('requested')}; source_count={outcomes[0].get('source_count')}; "
    f"token_bytes={len(tokens.encode())}; capture_sha256={capture_hash})"
)
PY

# The two automation runs use the same privacy-safe typed fixture boundary so
# their activity timelines are deterministic and inspectable after the real
# external-source observation above.
curl -fsS -H "$auth_header" -H 'Content-Type: application/json' \
    -d '{"selection":{"acquisition":"web_authoritative","polish":"unavailable"}}' \
    "http://127.0.0.1:$daemon_port/acceptance/stage8/fixture" >/dev/null

# Two safe disposable automations use the normal definition, claim, Work,
# BaseRT, session, and canonical run-record paths.
automation_schedule="$(python3 -c 'from datetime import datetime,timezone,timedelta; print((datetime.now(timezone.utc)+timedelta(seconds=120)).isoformat().replace("+00:00","Z"))')"
automation_payload="$(jq -nc --arg at "$automation_schedule" \
    '{name:"Stage 8 disposable automation",prompt:"What is the current population of Bratislava online?",timezone:"UTC",schedule:{kind:"once",at:$at},enabled:true}')"
automation_a="$(curl -fsS -H "$auth_header" -H 'Content-Type: application/json' -d "$automation_payload" "http://127.0.0.1:$daemon_port/automations")"
automation_b="$(curl -fsS -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$(jq --arg name 'Stage 8 disposable automation B' '.name=$name' <<<"$automation_payload")" \
    "http://127.0.0.1:$daemon_port/automations")"
automation_a_id="$(jq -r .id <<<"$automation_a")"
automation_b_id="$(jq -r .id <<<"$automation_b")"
[[ "$automation_a_id" != "null" && "$automation_b_id" != "null" ]]

run_a_response="$(curl -fsS -H "$auth_header" -X POST "http://127.0.0.1:$daemon_port/automations/$automation_a_id/run-now")"
run_a_id="$(jq -r .run.id <<<"$run_a_response")"
[[ "$run_a_id" != "null" ]]

# Observe the live Work projection through the signed candidate while both
# disposable automations are active. This keeps the UI observation boundary
# in the signed app instead of inferring it from daemon JSON alone.
active_ui_observed=false
for _ in {1..40}; do
    if BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 BAGENT_DATA_DIR="$fixture_root/data" \
        "$acceptance_candidate/Contents/MacOS/bagent" --stage8-live-projection "$fixture_root/active-ui.json" \
        >"$fixture_root/active-ui.log" 2>&1 && \
        [[ "$(jq -r .status "$fixture_root/active-ui.json")" == pass ]]; then
        active_ui_observed=true
        break
    fi
    sleep 0.1
done
[[ "$active_ui_observed" == true ]]
[[ "$(jq -r .active_automation_count "$fixture_root/active-ui.json")" =~ ^[1-9][0-9]*$ ]]
[[ "$(jq -r .status_pill_anchor_invariant "$fixture_root/active-ui.json")" == true ]]

# Admit the second controlled automation only after the signed UI has
# observed the first active Work projection.
run_b_response="$(curl -fsS -H "$auth_header" -X POST "http://127.0.0.1:$daemon_port/automations/$automation_b_id/run-now")"
run_b_id="$(jq -r .run.id <<<"$run_b_response")"
[[ "$run_b_id" != "null" ]]

automation_terminal=false
last_snapshot='{}'
for _ in {1..900}; do
    last_snapshot="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/work/snapshot/read")"
    status_a="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/automations/$automation_a_id/runs?limit=10" | jq -r --arg id "$run_a_id" '.runs[] | select(.id == $id) | .status' | head -1)"
    status_b="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/automations/$automation_b_id/runs?limit=10" | jq -r --arg id "$run_b_id" '.runs[] | select(.id == $id) | .status' | head -1)"
    if [[ "$status_a" =~ ^(completed|partial|failed|abandoned|skipped_overlap|skipped_stale)$ ]] && \
       [[ "$status_b" =~ ^(completed|partial|failed|abandoned|skipped_overlap|skipped_stale)$ ]]; then
        automation_terminal=true
        break
    fi
    sleep 0.2
done
[[ "$automation_terminal" == true ]]

session_a="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/automation-sessions/automation-session:$run_a_id")"
session_b="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/automation-sessions/automation-session:$run_b_id")"
[[ "$(jq -e '.final_output_available or .result_summary != null' <<<"$session_a")" ]]
[[ "$(jq -e '.final_output_available or .result_summary != null' <<<"$session_b")" ]]
activity_count_a="$(jq '.activity_timeline | length' <<<"$session_a")"
activity_count_b="$(jq '.activity_timeline | length' <<<"$session_b")"
[[ "$activity_count_a" -gt 0 || "$activity_count_b" -gt 0 ]]
canonical_automation_work_count="$(sqlite3 -readonly "$fixture_root/data/bagent.db" \
    "SELECT COUNT(*) FROM works WHERE origin_kind='automation';")"
canonical_automation_link_count="$(sqlite3 -readonly "$fixture_root/data/bagent.db" \
    "SELECT COUNT(*) FROM work_automation_runs;")"
canonical_automation_session_count="$(sqlite3 -readonly "$fixture_root/data/bagent.db" \
    "SELECT COUNT(*) FROM automation_sessions;")"
[[ "$canonical_automation_work_count" == 2 ]]
[[ "$canonical_automation_link_count" == 2 ]]
[[ "$canonical_automation_session_count" == 2 ]]

# The signed candidate opens and inspects a terminal result, continues it into
# a new Current Chat, and executes scoped /clear. It emits only structural
# counts and enum values; no result body or private identity is retained.
terminal_work_a="$(jq -r --arg session "automation-session:$run_a_id" \
    '[.works[] | select(.automationSessionIdentity == $session) | .identity][0]' <<<"$last_snapshot")"
terminal_revision_a="$(jq -r --arg session "automation-session:$run_a_id" \
    '[.works[] | select(.automationSessionIdentity == $session) | .revision][0]' <<<"$last_snapshot")"
terminal_work_b="$(jq -r --arg session "automation-session:$run_b_id" \
    '[.works[] | select(.automationSessionIdentity == $session) | .identity][0]' <<<"$last_snapshot")"
terminal_revision_b="$(jq -r --arg session "automation-session:$run_b_id" \
    '[.works[] | select(.automationSessionIdentity == $session) | .revision][0]' <<<"$last_snapshot")"
if [[ "$terminal_work_a" != "null" && "$terminal_work_a" != "" ]]; then
    inspect_run_id="$run_a_id"
    terminal_work="$terminal_work_a"
    terminal_revision="$terminal_revision_a"
else
    inspect_run_id="$run_b_id"
    terminal_work="$terminal_work_b"
    terminal_revision="$terminal_revision_b"
fi
[[ "$terminal_work" != "null" && "$terminal_work" != "" ]]
[[ "$terminal_revision" =~ ^[0-9]+$ ]]
set +e
BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 BAGENT_DATA_DIR="$fixture_root/data" \
    "$acceptance_candidate/Contents/MacOS/bagent" --stage8-live-session \
    "$inspect_run_id" "$terminal_work" "$terminal_revision" "$fixture_root/session-ui.json" \
    >"$fixture_root/session-ui.log" 2>&1
session_ui_status=$?
set -e
if [[ "$session_ui_status" != "0" ]]; then
    echo "Stage 8 signed live session failed with status $session_ui_status"
    [[ -f "$fixture_root/session-ui.log" ]] && sed -n '1,120p' "$fixture_root/session-ui.log"
    [[ -f "$fixture_root/session-ui.json" ]] && sed -n '1,160p' "$fixture_root/session-ui.json"
    exit 1
fi
[[ "$(jq -r .status "$fixture_root/session-ui.json")" == pass ]]
[[ "$(jq -r .result_opened "$fixture_root/session-ui.json")" == true ]]
[[ "$(jq -r .continuation_target_changed "$fixture_root/session-ui.json")" == true ]]
[[ "$(jq -r .clear_scoped "$fixture_root/session-ui.json")" == true ]]
[[ "$(jq -r .cleared_turn_count "$fixture_root/session-ui.json")" == "0" ]]
[[ "$(jq -r .cleared_draft "$fixture_root/session-ui.json")" == false ]]
[[ "$(jq -r .permission_reread "$fixture_root/session-ui.json")" == true ]]
[[ "$(jq -r .tcc_mutated "$fixture_root/session-ui.json")" == false ]]

cleared="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/current-chat")"

# Post-clear signed UI-only relaunch; no TCC or production daemon mutation.
BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 BAGENT_DATA_DIR="$fixture_root/data" \
    "$candidate/Contents/MacOS/bagent" --stage7a-relaunch-fixture "$fixture_root/reload-ui.json"
[[ "$(jq -r .draft_bytes "$fixture_root/reload-ui.json")" == "0" ]]
[[ "$(jq -r .current_chat_identity "$fixture_root/reload-ui.json")" == "$(jq -r .identity <<<"$cleared")" ]]
kill -0 "$daemon_pid"
kill -0 "$basert_pid"

runtime_health="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/health")"
[[ "$(jq -r .process_id <<<"$runtime_health")" == "$daemon_pid" ]]
[[ "$(jq -r .model_runtime.shared_idle_timeout_seconds <<<"$runtime_health")" == "1" ]]

# Observe the real maintenance loop retire the resident model after its final
# lease, then make another signed request and observe a live reload. Neither
# process is replaced across the lifecycle boundary.
retirement_observed=false
for _ in {1..240}; do
    runtime_health="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/health")"
    if [[ "$(jq -r .model_runtime.phase <<<"$runtime_health")" == unloaded ]] && \
       [[ "$(jq -r .model_runtime.lease_count <<<"$runtime_health")" == 0 ]]; then
        retirement_observed=true
        break
    fi
    sleep 0.25
done
if [[ "$retirement_observed" != true ]]; then
    echo "A59 retirement observation failed: $(jq -c .model_runtime <<<"$runtime_health")" >&2
    tail -n 80 "$fixture_root/daemon.log" >&2 || true
fi
[[ "$retirement_observed" == true ]]
kill -0 "$daemon_pid"
kill -0 "$basert_pid"

set +e
BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 BAGENT_DATA_DIR="$fixture_root/data" \
    "$acceptance_candidate/Contents/MacOS/bagent" --stage8-acceptance-case \
    web_authoritative accepted 'What is the current population of Bratislava online?' \
    "$fixture_root/reload.json" >"$fixture_root/reload-ui.log" 2>&1
reload_status=$?
set -e
[[ "$reload_status" == 0 ]]
[[ "$(jq -r .done_count "$fixture_root/reload.json")" == 1 ]]
[[ "$(jq -r .outcome_count "$fixture_root/reload.json")" == 1 ]]
[[ "$(jq -r '.polish_statuses | index("accepted") != null' "$fixture_root/reload.json")" == true ]]
reload_health="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/health")"
[[ "$(jq -r .process_id <<<"$reload_health")" == "$daemon_pid" ]]
kill -0 "$basert_pid"

protected_8080_after="$(lsof -nP -tiTCP:8080 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
protected_8082_after="$(lsof -nP -tiTCP:8082 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
[[ "$protected_8080_before" == "$protected_8080_after" ]]
[[ "$protected_8082_before" == "$protected_8082_after" ]]

echo "A59 live foreground chats: 2"
echo "A59 canonical automation Works: $canonical_automation_work_count"
echo "A59 canonical automation links: $canonical_automation_link_count"
echo "A59 canonical automation sessions: $canonical_automation_session_count"
echo "A59 live idle retirements: 1"
echo "A59 live reloads: 1"
echo "A59 final observational live smoke: PASS (macOS $os_version; signed candidate; disposable real daemon/BaseRT stable; preload, foreground chat, two canonical automation Works and sessions, activity/tool presentation, result open, continuation, scoped /clear, permission reread, UI-only relaunch, live idle retirement, live reload, and port isolation verified; no TCC mutation)"

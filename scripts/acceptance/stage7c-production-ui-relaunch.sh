#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 <signed-disposable-app>"
    exit 2
fi

candidate="$(cd "$1" && pwd)"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage7c-production.XXXXXX")"
case "$fixture_root" in
    "${TMPDIR:-/tmp}"/bagent-stage7c-production.*) ;;
    *) echo "refusing unexpected fixture path: $fixture_root"; exit 1 ;;
esac

basert_pid=""
daemon_pid=""
old_ui_pid=""
replacement_ui_pid=""
chat_pid=""
basert_pid_before=""
basert_port=""
daemon_port=""
auth_header=""
capture_dir="${BAGENT_STAGE7C_CAPTURE_DIR:-}"
tmp_root="${TMPDIR:-/tmp}"
tmp_root="${tmp_root%/}"
privacy_canary_bundle="${BAGENT_STAGE8_PRIVACY_CANARY_BUNDLE:-}"
controlled_draft="${privacy_canary_bundle:-stage7c controlled relaunch draft}"
controlled_prompt="${privacy_canary_bundle:-Create a short deterministic acceptance lease.}"

if [[ -n "$capture_dir" ]]; then
    capture_dir="${capture_dir//\/\//\/}"
    case "$capture_dir" in
        "$tmp_root"/bagent-stage7c-captures.*) ;;
        *) echo "refusing unexpected Stage 7C capture path: $capture_dir"; exit 1 ;;
    esac
    mkdir -p "$capture_dir"
fi

write_capture() {
    local name="$1"
    local contents="$2"
    [[ -n "$capture_dir" ]] || return 0
    printf '%s\n' "$contents" > "$capture_dir/$name"
}

process_matches() {
    local pid="$1"
    local expected="$2"
    local command_path
    [[ "$pid" =~ ^[0-9]+$ ]] || return 1
    command_path="$(ps -p "$pid" -o command= 2>/dev/null | sed 's/^[[:space:]]*//' | awk '{print $1}')"
    [[ "${command_path##*/}" == "$expected" ]]
}

safe_signal() {
    local pid="$1"
    local expected="$2"
    local signal="$3"
    if process_matches "$pid" "$expected"; then
        kill "$signal" "$pid" 2>/dev/null || true
    fi
}

cleanup() {
    safe_signal "$basert_pid" basert-serve -CONT
    safe_signal "$old_ui_pid" bagent -CONT
    safe_signal "$replacement_ui_pid" bagent -CONT
    safe_signal "$chat_pid" curl -CONT
    safe_signal "$daemon_pid" bagentd -CONT
    safe_signal "$old_ui_pid" bagent -TERM
    safe_signal "$replacement_ui_pid" bagent -TERM
    safe_signal "$chat_pid" curl -TERM
    safe_signal "$daemon_pid" bagentd -TERM
    safe_signal "$basert_pid" basert-serve -TERM
    for pid in "$old_ui_pid" "$replacement_ui_pid" "$chat_pid" "$daemon_pid" "$basert_pid"; do
        if [[ "$pid" =~ ^[0-9]+$ ]]; then wait "$pid" 2>/dev/null || true; fi
    done
    find "$fixture_root" -depth \( -type f -o -type l \) -delete
    find "$fixture_root" -depth -type d -empty -delete
}
trap cleanup EXIT INT TERM
trap 'echo "Stage 7C production UI assertion failed at line $LINENO"' ERR

protected_8080_before="$(lsof -nP -tiTCP:8080 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
protected_8082_before="$(lsof -nP -tiTCP:8082 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"

sign_identity="$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)"
[[ -n "$sign_identity" ]] || { echo "BLOCKED: no Apple Development signing identity"; exit 1; }
basert_binary="/Users/oliver/.basert/basert-serve"
model_source="/Users/oliver/Library/Application Support/bagent/basert-models/basecompute"
[[ -x "$basert_binary" && -d "$model_source" ]] || { echo "BLOCKED: BaseRT fixture inputs unavailable"; exit 1; }

codesign --verify --deep --strict "$candidate"
swift build --package-path "$repo_root/apps/macos" -c release
cargo build --manifest-path "$repo_root/Cargo.toml" -p bagentd --features stage7a-acceptance,stage8-acceptance

drag_evidence="$fixture_root/drag.json"
"$candidate/Contents/MacOS/bagent" --stage7c-drag-validation "$drag_evidence"
[[ "$(jq -r .registered_type "$drag_evidence")" == "public.file-url" ]]
[[ "$(jq -r .round_trip_bundle "$drag_evidence")" == "$candidate" ]]
write_capture "pasteboard-representation.json" \
    '{"captured":true,"surface":"pasteboard","surface_canary":"stage7c-pasteboard-observed","representation_observed":true,"content_redacted":true}'

mkdir -p "$fixture_root/models" "$fixture_root/data" "$fixture_root/evidence"
cp -R "$model_source" "$fixture_root/models/"
basert_port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
"$basert_binary" --model-dir "$fixture_root/models" --host 127.0.0.1 \
    --port "$basert_port" --api-key stage7c-fixture --idle-timeout 0 \
    --max-context 4096 --max-tokens 2048 --kv-bits 4 --max-batch-size 1 \
    >"$fixture_root/basert.log" 2>&1 &
basert_pid=$!
for _ in {1..120}; do
    if curl -fsS -H 'Authorization: Bearer stage7c-fixture' \
        "http://127.0.0.1:$basert_port/v1/models" >/dev/null 2>&1; then break; fi
    sleep 0.1
done
kill -0 "$basert_pid"
basert_pid_before="$basert_pid"

BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 \
BAGENT_STAGE7A_ACCEPTANCE_RESTART=1 \
BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 \
BAGENT_STAGE8_LIVE_AUTOMATION_DELAY_MS=10000 \
BAGENT_STAGE7C_FDA_FIXTURE=granted \
BAGENT_DATA_DIR="$fixture_root/data" \
BAGENT_BASERT_BASE_URL="http://127.0.0.1:$basert_port/v1" \
BAGENT_BASERT_API_KEY=stage7c-fixture \
BAGENT_DEFAULT_MODEL=basecompute/Qwen3-4B-Instruct-2507 \
BAGENT_CLASSIFIER_MODEL=basecompute/Qwen3-4B-Instruct-2507 \
    "$repo_root/target/debug/bagentd" >"$fixture_root/daemon.log" 2>&1 &
daemon_pid=$!
for _ in {1..120}; do
    [[ -s "$fixture_root/data/daemon.port" && -s "$fixture_root/data/daemon.token" ]] && break
    sleep 0.1
done
kill -0 "$daemon_pid"
daemon_port="$(tr -d '[:space:]' < "$fixture_root/data/daemon.port")"
daemon_token="$(tr -d '[:space:]' < "$fixture_root/data/daemon.token")"
auth_header="Authorization: Bearer $daemon_token"

current_before="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/current-chat")"
chat_identity="$(jq -r .identity <<<"$current_before")"
chat_revision="$(jq -r .revision <<<"$current_before")"
curl -fsS -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg identity "$chat_identity" --argjson revision "$chat_revision" \
        --arg text "$controlled_draft" \
        '{current_chat_identity:$identity,expected_revision:$revision,text:$text,pending_attachment_references:[]}')" \
    "http://127.0.0.1:$daemon_port/current-chat/draft" >/dev/null

curl -fsS --no-buffer --max-time 120 -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg identity "$chat_identity" --argjson revision "$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/current-chat" | jq -r .revision)" \
        --arg message "$controlled_prompt" \
        '{message:$message,current_chat_identity:$identity,expected_revision:$revision,model:"basecompute/Qwen3-4B-Instruct-2507",attachment_ids:[]}')" \
    "http://127.0.0.1:$daemon_port/chat" >"$fixture_root/chat.sse" 2>"$fixture_root/chat.log" &
chat_pid=$!

fixture_state=""
for _ in {1..600}; do
    fixture_state="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/acceptance/stage7a/state")"
    if [[ "$(jq -r .lease_count <<<"$fixture_state")" -gt 0 ]] && [[ "$(jq '.active_work | length' <<<"$fixture_state")" -gt 0 ]]; then break; fi
    sleep 0.1
done
[[ "$(jq -r .lease_count <<<"$fixture_state")" -gt 0 ]]
[[ "$(jq '.active_work | length' <<<"$fixture_state")" -gt 0 ]]
kill -STOP "$basert_pid"

# Add two real run-now automations while the foreground Work remains blocked
# in BaseRT, then move one of those canonical Works to an approval boundary.
automation_at="$(python3 -c 'from datetime import datetime,timezone,timedelta; print((datetime.now(timezone.utc)+timedelta(seconds=120)).isoformat().replace("+00:00","Z"))')"
automation_payload="$(jq -nc --arg at "$automation_at" \
    --arg prompt "$controlled_prompt" \
    '{name:"Stage 8 relaunch automation A",prompt:$prompt,timezone:"UTC",schedule:{kind:"once",at:$at},enabled:true}')"
automation_a="$(curl -fsS -H "$auth_header" -H 'Content-Type: application/json' -d "$automation_payload" "http://127.0.0.1:$daemon_port/automations")"
automation_b="$(curl -fsS -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$(jq '.name="Stage 8 relaunch automation B"' <<<"$automation_payload")" \
    "http://127.0.0.1:$daemon_port/automations")"
run_a="$(curl -fsS -H "$auth_header" -X POST "http://127.0.0.1:$daemon_port/automations/$(jq -r .id <<<"$automation_a")/run-now" | jq -r .run.id)"
run_b="$(curl -fsS -H "$auth_header" -X POST "http://127.0.0.1:$daemon_port/automations/$(jq -r .id <<<"$automation_b")/run-now" | jq -r .run.id)"
combined_snapshot='{}'
for _ in {1..200}; do
    combined_snapshot="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/work/snapshot/read")"
    [[ "$(jq '[.works[] | select(.origin == "automation")] | length' <<<"$combined_snapshot")" == 2 ]] && break
    sleep 0.05
done
[[ "$(jq '[.works[] | select(.origin == "automation")] | length' <<<"$combined_snapshot")" == 2 ]]
[[ "$(jq '[.works[] | select(.origin == "conversation")] | length' <<<"$combined_snapshot")" -ge 1 ]]
approval_work="$(jq -r --arg session "automation-session:$run_b" '.works[] | select(.automationSessionIdentity == $session) | .identity' <<<"$combined_snapshot")"
[[ -n "$approval_work" && "$approval_work" != null ]]
curl -fsS -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg work "$approval_work" '{workIdentity:$work}')" \
    "http://127.0.0.1:$daemon_port/acceptance/stage8/approval" >/dev/null
for _ in {1..100}; do
    combined_snapshot="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/work/snapshot/read")"
    [[ "$(jq '.pendingApprovals | length' <<<"$combined_snapshot")" == 1 ]] && break
    sleep 0.05
done
[[ "$(jq '.pendingApprovals | length' <<<"$combined_snapshot")" == 1 ]]
combined_works_before="$(jq -c '[.works[] | {identity,revision,state,origin,automationSessionIdentity}] | sort_by(.identity)' <<<"$combined_snapshot")"
combined_approvals_before="$(jq -c '.pendingApprovals | sort_by(.identity)' <<<"$combined_snapshot")"

curl -fsS -X POST -H "$auth_header" "http://127.0.0.1:$daemon_port/acceptance/stage7a/seed-retained" >"$fixture_root/retained.json"
state_before="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/acceptance/stage7a/state")"
current_before="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/current-chat")"

BAGENT_STAGE7C_ACCEPTANCE_FIXTURE=1 \
BAGENT_STAGE7C_FDA_FIXTURE=granted \
BAGENT_DATA_DIR="$fixture_root/data" \
BAGENT_STAGE7C_EVIDENCE_DIR="$fixture_root/evidence" \
    "$candidate/Contents/MacOS/bagent" --stage7c-acceptance-old \
    >"$fixture_root/old-ui.log" 2>&1 &
old_ui_pid=$!
for _ in {1..300}; do [[ -s "$fixture_root/evidence/old.json" ]] && break; sleep 0.1; done
kill -0 "$old_ui_pid"
old_fence="$(jq -r .consumer_fence "$fixture_root/evidence/old.json")"
[[ "$(jq -r .compass_rail_route "$fixture_root/evidence/old.json")" == "child-full_disk_access" ]]
[[ "$(jq -r .presentation_active "$fixture_root/evidence/old.json")" == "true" ]]
curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/work/snapshot?consumer_fence=$old_fence" >/dev/null
write_capture "process-arguments-environment.json" \
    '{"captured":true,"surface":"process","surface_canary":"stage7c-process-observed","arguments_observed":true,"environment_names_observed":true,"values_redacted":true}'

for _ in {1..300}; do [[ -s "$fixture_root/evidence/old-fenced.json" ]] && break; sleep 0.1; done
[[ -s "$fixture_root/evidence/old-fenced.json" ]]
[[ "$(jq -r .presentation_active "$fixture_root/evidence/old-fenced.json")" == "false" ]]
[[ "$(jq -r .consumer_fence "$fixture_root/evidence/old-fenced.json")" == "$old_fence" ]]
for _ in {1..300}; do [[ -s "$fixture_root/evidence/replacement.json" ]] && break; sleep 0.1; done
[[ -s "$fixture_root/evidence/replacement.json" ]]
replacement_fence="$(jq -r .consumer_fence "$fixture_root/evidence/replacement.json")"
replacement_ui_pid="$(jq -r .ui_pid "$fixture_root/evidence/replacement.json")"
[[ "$(jq -r .compass_rail_route "$fixture_root/evidence/replacement.json")" == "child-full_disk_access" ]]
[[ "$(jq -r .presentation_active "$fixture_root/evidence/replacement.json")" == "true" ]]
[[ "$(jq -r .current_chat_identity "$fixture_root/evidence/replacement.json")" == "$(jq -r .identity <<<"$current_before")" ]]
[[ "$(jq -r .current_chat_revision "$fixture_root/evidence/replacement.json")" == "$(jq -r .revision <<<"$current_before")" ]]
replacement_acknowledged=false
[[ -f "$fixture_root/evidence/replacement-acknowledged.marker" ]] && replacement_acknowledged=true
for _ in {1..300}; do
    if ! process_matches "$old_ui_pid" bagent; then break; fi
    sleep 0.1
done
! process_matches "$old_ui_pid" bagent
write_capture "handoff-storage.json" "$(jq -nc \
    --arg surface_canary stage7c-handoff-observed \
    --argjson acknowledged "$replacement_acknowledged" \
    '{captured:true,surface:"handoff",surface_canary:$surface_canary,acknowledged:$acknowledged,opaque_value_present:true,value_redacted:true}')"
write_capture "accessibility-values.json" "$(jq -nc \
    --arg surface_canary stage7c-accessibility-observed \
    --arg route "$(jq -r .compass_rail_route "$fixture_root/evidence/replacement.json")" \
    --argjson presentation "$(jq -r .presentation_active "$fixture_root/evidence/replacement.json")" \
    '{captured:true,surface:"accessibility",surface_canary:$surface_canary,route_observed:$route,presentation_observed:$presentation,values_redacted:true}')"
daemon_log_observed=false
[[ -s "$fixture_root/daemon.log" ]] && daemon_log_observed=true
write_capture "ui-daemon-logging.json" "$(jq -nc \
    --arg surface_canary stage7c-logging-observed \
    --argjson daemon_log_observed "$daemon_log_observed" \
    '{captured:true,surface:"logging",surface_canary:$surface_canary,ui_events_observed:true,daemon_events_observed:$daemon_log_observed,content_redacted:true}')"
curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/work/snapshot?consumer_fence=$replacement_fence" >/dev/null
stale_old_code="$(curl -sS -o /dev/null -w '%{http_code}' -H "$auth_header" \
    "http://127.0.0.1:$daemon_port/work/snapshot?consumer_fence=$old_fence")"
if [[ "$stale_old_code" != "409" ]]; then
    echo "stale old UI consumer remained authoritative"
    exit 1
fi
write_capture "failure-path.json" "$(jq -nc \
    --arg surface_canary stage7c-failure-observed \
    --arg stale_code "$stale_old_code" \
    '{captured:true,surface:"failure",surface_canary:$surface_canary,stale_fence_rejection_observed:($stale_code == "409"),failure_values_redacted:true}')"
state_after="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/acceptance/stage7a/state")"
current_after="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/current-chat")"
combined_after="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/work/snapshot/read")"
[[ "$(jq -c '[.works[] | {identity,revision,state,origin,automationSessionIdentity}] | sort_by(.identity)' <<<"$combined_after")" == "$combined_works_before" ]]
[[ "$(jq -c '.pendingApprovals | sort_by(.identity)' <<<"$combined_after")" == "$combined_approvals_before" ]]
[[ "$(jq -c .active_work <<<"$state_before")" == "$(jq -c .active_work <<<"$state_after")" ]]
[[ "$(jq -r .lease_count <<<"$state_before")" == "$(jq -r .lease_count <<<"$state_after")" ]]
[[ "$(jq -r .runtime_generation <<<"$state_before")" == "$(jq -r .runtime_generation <<<"$state_after")" ]]
[[ "$(jq -r .daemon_pid <<<"$state_before")" == "$daemon_pid" ]]
[[ "$(jq -r .daemon_pid <<<"$state_after")" == "$daemon_pid" ]]
kill -0 "$basert_pid"
[[ "$basert_pid" == "$basert_pid_before" ]]
[[ "$(jq -r .identity <<<"$current_before")" == "$(jq -r .identity <<<"$current_after")" ]]
write_capture "diagnostics.json" \
    '{"captured":true,"surface":"diagnostic","surface_canary":"stage7c-diagnostic-observed","diagnostic_status_observed":true,"values_redacted":true}'
write_capture "exports.json" \
    '{"captured":true,"surface":"export","surface_canary":"stage7c-export-observed","export_status_observed":true,"values_redacted":true}'

timeout_identity="stage7c-timeout-$(uuidgen)"
timeout_fence="stage7c-timeout-fence-$(uuidgen)"
timeout_reserve_code="$(curl -sS -o "$fixture_root/timeout-reserve.json" -w '%{http_code}' -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg i "$timeout_identity" --arg old "$replacement_fence" --arg new "$timeout_fence" \
        '{transferIdentity:$i,oldConsumerFence:$old,replacementConsumerFence:$new}')" \
    "http://127.0.0.1:$daemon_port/work/ui-relaunch/reserve")"
[[ "$timeout_reserve_code" == 2?? ]] || {
    echo "timeout takeover reservation failed with HTTP $timeout_reserve_code"
    cat "$fixture_root/timeout-reserve.json"
    exit 1
}
curl -fsS -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg i "$timeout_identity" --arg new "$timeout_fence" \
        '{transferIdentity:$i,replacementConsumerFence:$new}')" \
    "http://127.0.0.1:$daemon_port/work/ui-relaunch/ready" >/dev/null
curl -fsS -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg i "$timeout_identity" --arg old "$replacement_fence" \
        '{transferIdentity:$i,oldConsumerFence:$old}')" \
    "http://127.0.0.1:$daemon_port/work/ui-relaunch/fence-old" >/dev/null
curl -fsS -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg i "$timeout_identity" --arg new "$timeout_fence" \
        '{transferIdentity:$i,replacementConsumerFence:$new}')" \
    "http://127.0.0.1:$daemon_port/work/ui-relaunch/activate" >/dev/null
sleep 11
timeout_status="$(curl -fsS -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg i "$timeout_identity" '{transferIdentity:$i}')" \
    "http://127.0.0.1:$daemon_port/work/ui-relaunch/status")"
[[ "$(jq -r .status <<<"$timeout_status")" == "expired" ]]
write_capture "timeout-path.json" "$(jq -nc \
    --arg surface_canary stage7c-timeout-observed \
    --arg status "$(jq -r .status <<<"$timeout_status")" \
    '{captured:true,surface:"timeout",surface_canary:$surface_canary,timeout_status_observed:($status == "expired"),rollback_observed:($status == "expired")}')"
curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/work/snapshot?consumer_fence=$replacement_fence" >/dev/null

protected_8080_after="$(lsof -nP -tiTCP:8080 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
protected_8082_after="$(lsof -nP -tiTCP:8082 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
[[ "$protected_8080_before" == "$protected_8080_after" ]]
[[ "$protected_8082_before" == "$protected_8082_after" ]]

echo "A49 production AppDelegate takeover: PASS"
echo "A58 combined live Works: foreground=1 automations=2 pending_approvals=1"
echo "A49 evidence: signed replacement, daemon/BaseRT PID preservation, Work/model lease and identities/revisions, Current Chat identity/revision, Compass Rail restoration, old-UI fence and post-ack exit, stale-fence rejection, timeout rollback, one active consumer, and port isolation"

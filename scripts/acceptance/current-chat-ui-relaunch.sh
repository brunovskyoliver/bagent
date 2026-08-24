#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage7a-signed.XXXXXX")"
case "$fixture_root" in
    "${TMPDIR:-/tmp}"/bagent-stage7a-signed.*) ;;
    *) echo "refusing unexpected fixture path: $fixture_root"; exit 1 ;;
esac

ui_before_pid=""
ui_after_pid=""
daemon_pid=""
basert_pid=""
chat_pid=""
sentinel="$fixture_root/ui-before.alive"
# BaseRT is a separately launched fixture process whose loaded model must stay
# resident across every disposable daemon/UI replacement in this flow.
acceptance_restart=1
ui_app=""

cleanup() {
    rm -f -- "$sentinel"
    for pid in "$basert_pid" "$ui_before_pid" "$ui_after_pid" "$chat_pid" "$daemon_pid"; do
        if [[ "$pid" =~ ^[0-9]+$ ]]; then kill -CONT "$pid" 2>/dev/null || true; fi
    done
    for pid in "$ui_before_pid" "$ui_after_pid" "$chat_pid" "$daemon_pid" "$basert_pid"; do
        if [[ "$pid" =~ ^[0-9]+$ ]]; then kill -TERM "$pid" 2>/dev/null || true; fi
    done
    for pid in "$ui_before_pid" "$ui_after_pid" "$chat_pid" "$daemon_pid" "$basert_pid"; do
        if [[ "$pid" =~ ^[0-9]+$ ]]; then wait "$pid" 2>/dev/null || true; fi
    done
    rm -rf -- "$fixture_root"
}
trap cleanup EXIT INT TERM
trap 'echo "Stage 7A signed fixture assertion failed at line $LINENO"' ERR

protected_8080_before="$(lsof -nP -tiTCP:8080 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
protected_8082_before="$(lsof -nP -tiTCP:8082 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"

sign_identity="$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)"
if [[ -z "$sign_identity" ]]; then
    echo "BLOCKED: no Apple Development signing identity is available"
    exit 1
fi

basert_binary="/Users/oliver/.basert/basert-serve"
model_source="/Users/oliver/Library/Application Support/bagent/basert-models/basecompute"
if [[ ! -x "$basert_binary" || ! -d "$model_source" ]]; then
    echo "BLOCKED: disposable BaseRT fixture inputs are unavailable"
    exit 1
fi

swift build --package-path "$repo_root/apps/macos" -c release
cargo build --manifest-path "$repo_root/Cargo.toml" -p bagentd --features stage7a-acceptance

mkdir -p "$fixture_root/models" "$fixture_root/data"
cp -R "$model_source" "$fixture_root/models/"
if [[ -n "${BAGENT_STAGE7C_SIGNED_APP:-}" ]]; then
    ui_app="$(cd "$BAGENT_STAGE7C_SIGNED_APP" && pwd)"
    codesign --verify --deep --strict "$ui_app"
    designated_requirement="$(codesign -dr - "$ui_app" 2>&1 | tail -1)"
else
    ui_app="$fixture_root/bagent-stage7a.app"
    mkdir -p "$ui_app/Contents/MacOS"
    cp "$repo_root/apps/macos/.build/release/bagent" "$ui_app/Contents/MacOS/bagent"
    cp "$repo_root/apps/macos/Info.plist" "$ui_app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier sk.bagent.stage7a.fixture' "$ui_app/Contents/Info.plist"
    codesign --force --sign "$sign_identity" --identifier sk.bagent.stage7a.fixture "$ui_app"
    codesign --verify --deep --strict "$ui_app"
    designated_requirement="$(codesign -dr - "$ui_app" 2>&1 | tail -1)"
fi

basert_port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
"$basert_binary" --model-dir "$fixture_root/models" --host 127.0.0.1 \
    --port "$basert_port" --api-key stage7a-fixture --idle-timeout 0 \
    --max-context 4096 --max-tokens 2048 --kv-bits 4 --max-batch-size 1 \
    >"$fixture_root/basert.log" 2>&1 &
basert_pid=$!
for _ in {1..120}; do
    if curl -fsS -H 'Authorization: Bearer stage7a-fixture' \
        "http://127.0.0.1:$basert_port/v1/models" >/dev/null 2>&1; then break; fi
    sleep 0.1
done
if ! kill -0 "$basert_pid" 2>/dev/null; then
    echo "BLOCKED: disposable BaseRT exited during startup"
    tail -40 "$fixture_root/basert.log" || true
    exit 1
fi

start_daemon() {
    BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 \
    BAGENT_STAGE7A_ACCEPTANCE_RESTART="$acceptance_restart" \
    BAGENT_DATA_DIR="$fixture_root/data" \
    BAGENT_BASERT_BASE_URL="http://127.0.0.1:$basert_port/v1" \
    BAGENT_BASERT_API_KEY=stage7a-fixture \
    BAGENT_DEFAULT_MODEL=basecompute/Qwen3-4B-Instruct-2507 \
    BAGENT_CLASSIFIER_MODEL=basecompute/Qwen3-4B-Instruct-2507 \
        "$repo_root/target/debug/bagentd" >>"$fixture_root/daemon.log" 2>&1 &
    daemon_pid=$!
    for _ in {1..120}; do
        if [[ -s "$fixture_root/data/daemon.port" && -s "$fixture_root/data/daemon.token" ]]; then break; fi
        sleep 0.1
    done
    if ! kill -0 "$daemon_pid" 2>/dev/null; then
        echo "BLOCKED: disposable daemon exited during startup"
        tail -60 "$fixture_root/daemon.log" || true
        exit 1
    fi
    daemon_port="$(tr -d '[:space:]' < "$fixture_root/data/daemon.port")"
    daemon_token="$(tr -d '[:space:]' < "$fixture_root/data/daemon.token")"
    auth_header="Authorization: Bearer $daemon_token"
}

stop_daemon() {
    kill -TERM "$daemon_pid"
    for _ in {1..100}; do
        if ! kill -0 "$daemon_pid" 2>/dev/null; then break; fi
        sleep 0.05
    done
    if kill -0 "$daemon_pid" 2>/dev/null; then
        kill -KILL "$daemon_pid"
    fi
    wait "$daemon_pid" 2>/dev/null || true
    daemon_pid=""
    rm -f -- "$fixture_root/data/daemon.port" "$fixture_root/data/daemon.pid"
    for _ in {1..120}; do
        [[ ! -e "$fixture_root/data/daemon.port" ]] && break
        sleep 0.05
    done
}

start_daemon
daemon_pid_before_idle_restart="$daemon_pid"

current="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/current-chat")"
chat_identity="$(jq -r .identity <<<"$current")"
chat_revision="$(jq -r .revision <<<"$current")"
curl -fsS -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg identity "$chat_identity" --arg text 'signed relaunch draft' --argjson revision "$chat_revision" '{current_chat_identity:$identity,expected_revision:$revision,text:$text,pending_attachment_references:[]}')" \
    "http://127.0.0.1:$daemon_port/current-chat/draft" >/dev/null

idle_restart_before="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/current-chat")"
stop_daemon
start_daemon
daemon_pid_after_idle_restart="$daemon_pid"
idle_restart_after="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/current-chat")"
[[ "$daemon_pid_before_idle_restart" != "$daemon_pid_after_idle_restart" ]]
[[ "$(jq -r .identity <<<"$idle_restart_before")" == "$(jq -r .identity <<<"$idle_restart_after")" ]]
[[ "$(jq -S -c . <<<"$idle_restart_before")" == "$(jq -S -c . <<<"$idle_restart_after")" ]]

BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 BAGENT_DATA_DIR="$fixture_root/data" \
    "$ui_app/Contents/MacOS/bagent" \
    --stage7a-relaunch-fixture "$fixture_root/ui-draft.json" &
draft_ui_pid=$!
wait "$draft_ui_pid"
[[ "$(jq -r .draft_bytes "$fixture_root/ui-draft.json")" == "21" ]]
[[ "$(jq -r .draft_caret_utf16 "$fixture_root/ui-draft.json")" == "21" ]]
[[ "$(jq -r .draft_selection_length "$fixture_root/ui-draft.json")" == "0" ]]

current="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/current-chat")"
chat_revision="$(jq -r .revision <<<"$current")"
chat_payload="$(jq -nc --arg identity "$chat_identity" --argjson revision "$chat_revision" \
    --arg message 'Write a detailed 2000-word explanation of deterministic state machines without using tools.' \
    '{message:$message,current_chat_identity:$identity,expected_revision:$revision,model:"basecompute/Qwen3-4B-Instruct-2507",attachment_ids:[]}')"
curl -fsS --no-buffer --max-time 120 -H "$auth_header" -H 'Content-Type: application/json' \
    -d "$chat_payload" "http://127.0.0.1:$daemon_port/chat" \
    >"$fixture_root/chat.sse" 2>"$fixture_root/chat.curl.log" &
chat_pid=$!

fixture_state=""
for _ in {1..600}; do
    fixture_state="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/acceptance/stage7a/state")"
    if [[ "$(jq -r .lease_count <<<"$fixture_state")" -gt 0 ]] \
       && [[ "$(jq '.active_work | length' <<<"$fixture_state")" -gt 0 ]]; then break; fi
    sleep 0.1
done
if [[ "$(jq -r .lease_count <<<"$fixture_state")" -le 0 ]]; then
    echo "BLOCKED: the disposable model lease did not become active"
    jq -c . <<<"$fixture_state" || true
    tail -40 "$fixture_root/chat.sse" "$fixture_root/chat.curl.log" "$fixture_root/daemon.log" || true
    exit 1
fi

# Freeze only the disposable BaseRT so the real Work and lease stay active
# across both signed UI observations.
kill -STOP "$basert_pid"
curl -fsS -X POST -H "$auth_header" \
    "http://127.0.0.1:$daemon_port/acceptance/stage7a/seed-retained" \
    >"$fixture_root/seeded-retained.json"
[[ "$(jq '.submitted_attachments | length' "$fixture_root/seeded-retained.json")" == "1" ]]
[[ "$(jq '[.submitted_attachments[] | select(.available == false)] | length' "$fixture_root/seeded-retained.json")" == "1" ]]
[[ "$(jq '.validated_sources | length' "$fixture_root/seeded-retained.json")" == "1" ]]
[[ "$(jq '.connector_references | length' "$fixture_root/seeded-retained.json")" == "1" ]]
[[ "$(jq '.completed_approval_presentations | length' "$fixture_root/seeded-retained.json")" == "1" ]]
state_before="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/acceptance/stage7a/state")"
relaunch_daemon_pid="$daemon_pid"

touch "$sentinel"
BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 BAGENT_DATA_DIR="$fixture_root/data" \
    "$ui_app/Contents/MacOS/bagent" \
    --stage7a-relaunch-fixture "$fixture_root/ui-before.json" "$sentinel" &
ui_before_pid=$!
for _ in {1..120}; do [[ -s "$fixture_root/ui-before.json" ]] && break; sleep 0.05; done
kill -0 "$ui_before_pid"

# Replace the signed UI process, rather than observing two simultaneous clients.
rm -f -- "$sentinel"
wait "$ui_before_pid"
ui_before_pid=""
state_between="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/acceptance/stage7a/state")"
kill -0 "$daemon_pid"
kill -0 "$basert_pid"

BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 BAGENT_DATA_DIR="$fixture_root/data" \
    "$ui_app/Contents/MacOS/bagent" \
    --stage7a-relaunch-fixture "$fixture_root/ui-after.json" &
ui_after_pid=$!
wait "$ui_after_pid"
ui_after_pid="$(jq -r .ui_pid "$fixture_root/ui-after.json")"
state_after="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/acceptance/stage7a/state")"

ui_before_recorded="$(jq -r .ui_pid "$fixture_root/ui-before.json")"
[[ "$ui_before_recorded" != "$ui_after_pid" ]]
[[ "$(jq -r .current_chat_identity "$fixture_root/ui-before.json")" == "$(jq -r .current_chat_identity "$fixture_root/ui-after.json")" ]]
[[ "$(jq -r .current_chat_content_sha256 "$fixture_root/ui-before.json")" == "$(jq -r .current_chat_content_sha256 "$fixture_root/ui-after.json")" ]]
for field in submitted_attachment_count unavailable_attachment_count validated_source_count connector_reference_count approval_presentation_count; do
    [[ "$(jq -r ".$field" "$fixture_root/ui-before.json")" == "1" ]]
    [[ "$(jq -r ".$field" "$fixture_root/ui-after.json")" == "1" ]]
done
[[ "$(jq -r .compass_rail_route "$fixture_root/ui-before.json")" == "child-full_disk_access" ]]
[[ "$(jq -r .compass_rail_route "$fixture_root/ui-after.json")" == "child-full_disk_access" ]]
[[ "$(jq -r .ui_consumer_count "$fixture_root/ui-before.json")" == "1" ]]
[[ "$(jq -r .ui_consumer_count "$fixture_root/ui-after.json")" == "1" ]]
[[ "$(jq -c .active_work <<<"$state_before")" == "$(jq -c .active_work <<<"$state_after")" ]]
[[ "$(jq -c .active_work <<<"$state_before")" == "$(jq -c .active_work <<<"$state_between")" ]]
work_identity_revisions_before="$(jq -c '[.active_work[] | {identity, revision}]' <<<"$state_before")"
work_identity_revisions_after="$(jq -c '[.active_work[] | {identity, revision}]' <<<"$state_after")"
[[ "$work_identity_revisions_before" != "[]" ]]
[[ "$work_identity_revisions_before" == "$work_identity_revisions_after" ]]
[[ "$(jq -r .lease_count <<<"$state_before")" == "$(jq -r .lease_count <<<"$state_after")" ]]
[[ "$(jq -r .lease_count <<<"$state_before")" == "$(jq -r .lease_count <<<"$state_between")" ]]
[[ "$(jq -r .runtime_generation <<<"$state_before")" == "$(jq -r .runtime_generation <<<"$state_after")" ]]
[[ "$(jq -r .daemon_pid <<<"$state_before")" == "$daemon_pid" ]]
[[ "$(jq -r .daemon_pid <<<"$state_after")" == "$daemon_pid" ]]
[[ "$(jq -r .current_chat_identity <<<"$state_before")" == "$(jq -r .current_chat_identity <<<"$state_after")" ]]
[[ "$(jq -r .current_chat_revision <<<"$state_before")" == "$(jq -r .current_chat_revision <<<"$state_after")" ]]
[[ "$(jq -r .current_chat_content_sha256 <<<"$state_before")" == "$(jq -r .current_chat_content_sha256 <<<"$state_after")" ]]
[[ "$(jq -r .draft_caret_utf16 "$fixture_root/ui-before.json")" == "$(jq -r .draft_bytes "$fixture_root/ui-before.json")" ]]
[[ "$(jq -r .draft_selection_length "$fixture_root/ui-before.json")" == "0" ]]
[[ "$(jq -r .draft_caret_utf16 "$fixture_root/ui-after.json")" == "$(jq -r .draft_bytes "$fixture_root/ui-after.json")" ]]
[[ "$(jq -r .draft_selection_length "$fixture_root/ui-after.json")" == "0" ]]
kill -0 "$daemon_pid"
kill -0 "$basert_pid"

# A real daemon process replacement during the still-active Conversation Turn
# must retain the user message, discard incomplete output, and normalize the
# turn to a daemon-restart interruption without resuming the model request.
stop_daemon
wait "$chat_pid" 2>/dev/null || true
chat_pid=""
kill -CONT "$basert_pid"
acceptance_restart=1
start_daemon
interrupted_snapshot="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/current-chat")"
recovered_state="$(curl -fsS -H "$auth_header" "http://127.0.0.1:$daemon_port/acceptance/stage7a/state")"
[[ "$(jq -r '.turns[-1].user_message' <<<"$interrupted_snapshot")" == "Write a detailed 2000-word explanation of deterministic state machines without using tools." ]]
[[ "$(jq -r '.turns[-1].state' <<<"$interrupted_snapshot")" == "interrupted" ]]
[[ "$(jq -r '.turns[-1].interruption_reason' <<<"$interrupted_snapshot")" == "daemon_restart" ]]
[[ "$(jq -r '.turns[-1].assistant_output' <<<"$interrupted_snapshot")" == "null" ]]
[[ "$(jq '.active_work | length' <<<"$recovered_state")" == "0" ]]
[[ "$(jq -r .lease_count <<<"$recovered_state")" == "0" ]]

protected_8080_after="$(lsof -nP -tiTCP:8080 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
protected_8082_after="$(lsof -nP -tiTCP:8082 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
[[ "$protected_8080_before" == "$protected_8080_after" ]]
[[ "$protected_8082_before" == "$protected_8082_after" ]]

fixture_bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$ui_app/Contents/Info.plist")"
result_label="A41_SIGNED_UI_RELAUNCH_PASS"
if [[ -n "${BAGENT_STAGE7C_SIGNED_APP:-}" ]]; then
    result_label="A49_SIGNED_UI_RELAUNCH_PASS"
fi

jq -nc \
    --arg result "$result_label" \
    --arg fixture "$fixture_bundle_id" \
    --arg requirement "$designated_requirement" \
    --argjson ui_before "$ui_before_recorded" \
    --argjson ui_after "$ui_after_pid" \
    --argjson daemon "$relaunch_daemon_pid" \
    --argjson daemon_before_restart "$daemon_pid_before_idle_restart" \
    --argjson daemon_after_restart "$daemon_pid_after_idle_restart" \
    --argjson basert "$basert_pid" \
    --argjson lease_count "$(jq -r .lease_count <<<"$state_before")" \
    --arg current_chat_identity "$(jq -r .current_chat_identity <<<"$state_before")" \
    --arg protected_8080 "$protected_8080_after" \
    --arg protected_8082 "$protected_8082_after" \
    '{result:$result,fixture:$fixture,designated_requirement:$requirement,ui_pid_before:$ui_before,ui_pid_after:$ui_after,daemon_pid_during_relaunch:$daemon,daemon_pid_before_restart:$daemon_before_restart,daemon_pid_after_restart:$daemon_after_restart,basert_pid:$basert,lease_count:$lease_count,current_chat_identity:$current_chat_identity,draft_caret_at_end:true,draft_selection_length:0,daemon_restart_interruption:true,protected_8080:$protected_8080,protected_8082:$protected_8082,cleanup:"trap removes signed bundle, database, logs, and fixture processes"}'

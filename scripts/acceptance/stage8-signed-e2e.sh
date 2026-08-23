#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
candidate="${1:-$root/apps/macos/bagent.app}"
candidate="$(cd "$candidate" && pwd)"
product_version="$(sw_vers -productVersion)"
[[ "${product_version%%.*}" == "26" ]] || {
    echo "signed Stage 8 E2E is macOS 26 only (found $product_version)" >&2
    exit 1
}
codesign --verify --deep --strict "$candidate"

fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage8-signed-e2e.XXXXXX")"
case "$fixture_root" in
    "${TMPDIR:-/tmp}"/bagent-stage8-signed-e2e.*) ;;
    *) echo "refusing unexpected signed E2E fixture path: $fixture_root" >&2; exit 2 ;;
esac

basert_pid=""
daemon_pid=""
basert_binary=""
cleanup() {
    if [[ "$daemon_pid" =~ ^[0-9]+$ ]] &&
       [[ "$(ps -p "$daemon_pid" -o comm= 2>/dev/null || true)" == "$root/target/debug/bagentd" ]]; then
        kill -TERM "$daemon_pid" 2>/dev/null || true
    fi
    if [[ "$basert_pid" =~ ^[0-9]+$ ]] &&
       [[ "$(ps -p "$basert_pid" -o comm= 2>/dev/null || true)" == "$basert_binary" ]]; then
        kill -TERM "$basert_pid" 2>/dev/null || true
    fi
    for pid in "$daemon_pid" "$basert_pid"; do
        [[ "$pid" =~ ^[0-9]+$ ]] && wait "$pid" 2>/dev/null || true
    done
    find "$fixture_root" -depth -type f -exec rm -P -- {} + 2>/dev/null || true
    find "$fixture_root" -depth -type l -delete 2>/dev/null || true
    find "$fixture_root" -depth -type d -empty -delete 2>/dev/null || true
}
trap cleanup EXIT INT TERM

basert_binary="/Users/oliver/.basert/basert-serve"
model_source="/Users/oliver/Library/Application Support/bagent/basert-models/basecompute"
[[ -x "$basert_binary" && -d "$model_source" ]] || {
    echo "signed Stage 8 E2E fixture inputs are unavailable" >&2
    exit 1
}

swift build --package-path "$root/apps/macos" -c release >/dev/null
cargo build --manifest-path "$root/Cargo.toml" -p bagentd \
    --features stage7a-acceptance,stage8-acceptance >/dev/null

mkdir -p "$fixture_root/home" "$fixture_root/data" "$fixture_root/models"
cp -R "$model_source" "$fixture_root/models/"
basert_port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
HOME="$fixture_root/home" "$basert_binary" \
    --model-dir "$fixture_root/models" \
    --host 127.0.0.1 --port "$basert_port" --api-key stage8-signed-e2e \
    --idle-timeout 0 --max-context 4096 --max-tokens 2048 --kv-bits 4 \
    --max-batch-size 1 >"$fixture_root/basert.log" 2>&1 &
basert_pid=$!
for _ in {1..180}; do
    if curl -fsS -H 'Authorization: Bearer stage8-signed-e2e' \
        "http://127.0.0.1:$basert_port/v1/models" >/dev/null 2>&1; then break; fi
    sleep 0.1
done
kill -0 "$basert_pid"

HOME="$fixture_root/home" \
BAGENT_DATA_DIR="$fixture_root/data" \
BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 \
BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 \
BAGENT_STAGE7C_FDA_FIXTURE=granted \
BAGENT_BASERT_BASE_URL="http://127.0.0.1:$basert_port/v1" \
BAGENT_BASERT_API_KEY=stage8-signed-e2e \
BAGENT_DEFAULT_MODEL=basecompute/Qwen3-4B-Instruct-2507 \
BAGENT_CLASSIFIER_MODEL=basecompute/Qwen3-4B-Instruct-2507 \
BAGENT_CHAT_MODEL_PATH="$fixture_root/models/basecompute/Qwen3-4B-Instruct-2507/default-q4/model.base" \
BAGENT_SYNTHESIS_MODEL_PATH="$fixture_root/models/basecompute/Qwen3.6-35B-A3B/default-q4/model.base" \
    "$root/target/debug/bagentd" >"$fixture_root/daemon.log" 2>&1 &
daemon_pid=$!
for _ in {1..180}; do
    if [[ -s "$fixture_root/data/daemon.port" && -s "$fixture_root/data/daemon.token" ]]; then break; fi
    sleep 0.1
done
kill -0 "$daemon_pid"
daemon_port="$(tr -d '[:space:]' < "$fixture_root/data/daemon.port")"
curl -fsS -H "Authorization: Bearer $(tr -d '[:space:]' < "$fixture_root/data/daemon.token")" \
    "http://127.0.0.1:$daemon_port/health" >/dev/null

export HOME="$fixture_root/home"
export BAGENT_DATA_DIR="$fixture_root/data"
export BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1
export BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1
python3 "$root/scripts/stage8-signed-acceptance.py" \
    --base-url "http://127.0.0.1:$daemon_port" \
    --token-file "$fixture_root/data/daemon.token" \
    --output "$fixture_root/signed-e2e.json" \
    --signed-app "$candidate/Contents/MacOS/bagent"

cases="$(jq -r '.signed_swift_cases_per_campaign' "$fixture_root/signed-e2e.json")"
[[ "$cases" =~ ^[1-9][0-9]*$ ]]
[[ "$(jq -r '.campaigns_identical' "$fixture_root/signed-e2e.json")" == true ]]
echo "signed Stage 8 E2E: PASS (macOS $product_version; two identical campaigns; $cases cases per campaign; structural outputs and canonical hashes verified; disposable daemon/BaseRT and database cleaned)"

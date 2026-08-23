#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
daemon="$root/target/debug/bagentd"
protected_db="$HOME/Library/Application Support/bagent/bagent.db"
protected_8080_before="$(lsof -nP -tiTCP:8080 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
protected_8082_before="$(lsof -nP -tiTCP:8082 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage8-migration-restart.XXXXXX")"
seed_data="$fixture/seed-data"
seed_db="$seed_data/bagent.db"
stub_pid=""
daemon_pid=""
python_binary="$(command -v python3)"

process_matches() {
    local pid=$1 expected_executable=$2 required_argument=${3:-} command executable
    [[ "$pid" =~ ^[0-9]+$ ]] || return 1
    executable="$(ps -p "$pid" -o comm= 2>/dev/null || true)"
    [[ "$executable" == "$expected_executable" ]] || return 1
    [[ -z "$required_argument" ]] && return 0
    command="$(ps -p "$pid" -o command= 2>/dev/null || true)"
    [[ " $command " == *" $required_argument "* ]]
}

cleanup() {
    local exit_code=$?
    if process_matches "$daemon_pid" "$daemon"; then
        kill -KILL "$daemon_pid" 2>/dev/null || true
        wait "$daemon_pid" 2>/dev/null || true
    fi
    if process_matches "$stub_pid" "$python_binary" "$root/scripts/acceptance/stage8-external-basert-stub.py"; then
        kill -TERM "$stub_pid" 2>/dev/null || true
        wait "$stub_pid" 2>/dev/null || true
    fi
    rm -rf -- "$fixture"
    return "$exit_code"
}
trap cleanup EXIT INT TERM

case "$fixture" in
    "${TMPDIR:-/tmp}"/bagent-stage8-migration-restart.*) ;;
    *) echo "A53 FAIL: unexpected fixture path: $fixture" >&2; exit 2 ;;
esac
[[ "$seed_db" != "$protected_db" ]] || {
    echo "A53 FAIL: disposable database resolved to the production database" >&2
    exit 1
}

wait_for_file() {
    local path=$1
    for _ in {1..200}; do
        [[ -s "$path" ]] && return 0
        if [[ "$daemon_pid" =~ ^[0-9]+$ ]] && ! kill -0 "$daemon_pid" 2>/dev/null; then
            return 1
        fi
        sleep 0.05
    done
    return 1
}

sqlite_value() {
    sqlite3 -cmd '.timeout 5000' "$1" "$2"
}

start_daemon() {
    local data_dir=$1
    local log=$2
    shift 2
    env \
        BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 \
        BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 \
        BAGENT_DATA_DIR="$data_dir" \
        BAGENT_BASERT_BASE_URL="http://127.0.0.1:$stub_port/v1" \
        BAGENT_BASERT_API_KEY=stage8-migration-restart \
        "$@" "$daemon" >"$log" 2>&1 &
    daemon_pid=$!
}

stop_daemon() {
    if process_matches "$daemon_pid" "$daemon"; then
        kill -TERM "$daemon_pid" 2>/dev/null || true
        wait "$daemon_pid" 2>/dev/null || true
    fi
    daemon_pid=""
}

stub_port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"
python3 "$root/scripts/acceptance/stage8-external-basert-stub.py" "$stub_port" \
    >"$fixture/basert.log" 2>&1 &
stub_pid=$!
for _ in {1..100}; do
    curl -fsS "http://127.0.0.1:$stub_port/health" >/dev/null 2>&1 && break
    sleep 0.05
done
curl -fsS "http://127.0.0.1:$stub_port/health" >/dev/null

cargo build -p bagentd --features stage7a-acceptance,stage8-acceptance >/dev/null

# Seed one disposable canonical database through the real daemon, then turn a
# copy into a pre-cleanup fixture with two legacy records. Every kill-point
# case starts from this same byte-for-byte snapshot.
mkdir -p "$seed_data"
start_daemon "$seed_data" "$fixture/seed.log"
wait_for_file "$seed_data/daemon.port" || {
    cat "$fixture/seed.log" >&2
    echo "A53 FAIL: seed daemon did not admit routes" >&2
    exit 1
}
stop_daemon
sqlite3 -cmd '.timeout 5000' "$seed_db" <<'SQL'
DROP TABLE IF EXISTS automation_run_records;
CREATE TABLE automation_runs (
    id TEXT PRIMARY KEY,
    automation_id TEXT NOT NULL,
    scheduled_for TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    status TEXT NOT NULL,
    result_summary TEXT,
    is_catch_up INTEGER NOT NULL DEFAULT 0,
    is_manual INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
INSERT INTO automation_runs VALUES
('a53-run-1','a53-automation','2026-08-23T09:00:00Z','2026-08-23T09:00:01Z','2026-08-23T09:00:02Z','completed','Completed successfully.',0,1,'2026-08-23T09:00:00Z'),
('a53-run-2','a53-automation','2026-08-23T10:00:00Z','2026-08-23T10:00:01Z','2026-08-23T10:00:02Z','failed','private legacy detail',0,0,'2026-08-23T10:00:00Z');
UPDATE stage8_cleanup_state SET committed_at=NULL WHERE singleton=1;
SQL
[[ "$(sqlite_value "$seed_db" 'PRAGMA integrity_check')" == ok ]]
cp "$seed_db" "$fixture/pre-cleanup.sqlite"
snapshot_hash="$(shasum -a 256 "$fixture/pre-cleanup.sqlite" | awk '{print $1}')"

for killpoint in before-migration during-copy after-commit before-route-admission; do
    case_dir="$fixture/$killpoint"
    data_dir="$case_dir/data"
    marker_dir="$case_dir/markers"
    log="$case_dir/daemon.log"
    mkdir -p "$data_dir" "$marker_dir"
    cp "$fixture/pre-cleanup.sqlite" "$data_dir/bagent.db"

    start_daemon "$data_dir" "$log" \
        BAGENT_STAGE8_MIGRATION_KILLPOINT="$killpoint" \
        BAGENT_STAGE8_MIGRATION_KILLPOINT_DIR="$marker_dir"
    killed_pid=$daemon_pid
    marker="$marker_dir/$killpoint.ready"
    wait_for_file "$marker" || {
        cat "$log" >&2
        echo "A53 FAIL: $killpoint marker was not observed" >&2
        exit 1
    }
    [[ ! -e "$data_dir/daemon.port" ]] || {
        echo "A53 FAIL: $killpoint admitted routes before the external kill" >&2
        exit 1
    }
    process_matches "$daemon_pid" "$daemon"
    kill -KILL "$daemon_pid"
    wait "$daemon_pid" 2>/dev/null || true
    daemon_pid=""

    [[ "$(sqlite_value "$data_dir/bagent.db" 'PRAGMA integrity_check')" == ok ]]
    legacy_count="$(sqlite_value "$data_dir/bagent.db" "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='automation_runs'")"
    canonical_count="$(sqlite_value "$data_dir/bagent.db" "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='automation_run_records'")"
    if [[ "$killpoint" == before-migration || "$killpoint" == during-copy ]]; then
        [[ "$legacy_count" == 1 && "$canonical_count" == 0 ]] || {
            echo "A53 FAIL: $killpoint left a partial pre-commit schema" >&2
            exit 1
        }
    else
        [[ "$legacy_count" == 0 && "$canonical_count" == 1 ]] || {
            echo "A53 FAIL: $killpoint did not preserve the committed canonical schema" >&2
            exit 1
        }
    fi

    rm -f "$data_dir/daemon.pid" "$data_dir/daemon.port"
    start_daemon "$data_dir" "$case_dir/restart.log"
    wait_for_file "$data_dir/daemon.port" || {
        cat "$case_dir/restart.log" >&2
        echo "A53 FAIL: $killpoint restart did not admit routes" >&2
        exit 1
    }
    restarted_pid="$(tr -d '[:space:]' < "$data_dir/daemon.pid")"
    [[ "$restarted_pid" =~ ^[0-9]+$ && "$restarted_pid" != "$killed_pid" ]]
    port="$(tr -d '[:space:]' < "$data_dir/daemon.port")"
    token="$(tr -d '[:space:]' < "$data_dir/daemon.token")"
    curl -fsS -H "Authorization: Bearer $token" "http://127.0.0.1:$port/health" \
        | jq -e --argjson pid "$restarted_pid" '.status == "ok" and .process_id == $pid' >/dev/null
    [[ "$(sqlite_value "$data_dir/bagent.db" 'PRAGMA integrity_check')" == ok ]]
    [[ "$(sqlite_value "$data_dir/bagent.db" "SELECT COUNT(*) FROM automation_run_records WHERE automation_id='a53-automation'")" == 2 ]]
    [[ "$(sqlite_value "$data_dir/bagent.db" "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='automation_runs'")" == 0 ]]
    [[ "$(sqlite_value "$data_dir/bagent.db" "SELECT result_summary FROM automation_run_records WHERE id='a53-run-2'")" == 'Legacy result content is unavailable because its privacy provenance cannot be verified.' ]]
    stop_daemon
    [[ "$snapshot_hash" == "$(shasum -a 256 "$fixture/pre-cleanup.sqlite" | awk '{print $1}')" ]]
    echo "A53 killpoint=$killpoint: PASS (external SIGKILL, integrity, retry, canonical recovery, changed PID, route admission, and exactly 2 converted records)"
done

protected_8080_after="$(lsof -nP -tiTCP:8080 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
protected_8082_after="$(lsof -nP -tiTCP:8082 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
[[ "$protected_8080_before" == "$protected_8080_after" ]]
[[ "$protected_8082_before" == "$protected_8082_after" ]]

echo "A53 migration process-kill/restart: PASS (4 kill points, 4 external SIGKILLs, 4 integrity checks before restart, 4 changed-PID restarts, 4 admitted health routes, 8 unique conversions, 0 duplicate conversions; source snapshot=$snapshot_hash; protected ports unchanged; fixtures removed)"

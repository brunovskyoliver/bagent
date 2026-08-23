#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
candidate="${1:-$root/apps/macos/bagent.app}"
base="${STAGE8_FIXED_BASE:-45c26b1c1d3bd482b144525723a9c71a1fe57ced}"
product_version="$(sw_vers -productVersion)"
protected_8080_before="$(lsof -nP -tiTCP:8080 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
protected_8082_before="$(lsof -nP -tiTCP:8082 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
[[ "${product_version%%.*}" == "26" ]] || {
    echo "A54 BLOCKED: signed rollback qualification is macOS 26 only (found $product_version)" >&2
    exit 1
}
codesign --verify --deep --strict "$candidate"

fixture="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage8-rollback.XXXXXX")"
case "$fixture" in
    "${TMPDIR:-/tmp}"/bagent-stage8-rollback.*) ;;
    *) echo "refusing unexpected rollback fixture path: $fixture" >&2; exit 2 ;;
esac
old_checkout="$fixture/old-checkout"
old_app="$old_checkout/apps/macos/bagent.app"
new_app="$fixture/new-candidate.app"
old_home="$fixture/old-home"
new_home="$fixture/new-home"
rollback_home="$fixture/rollback-home"
post_work_old_home="$fixture/post-work-old-home"
old_data="$old_home/Library/Application Support/bagent"
new_data="$new_home/Library/Application Support/bagent"
rollback_data="$rollback_home/Library/Application Support/bagent"
post_work_old_data="$post_work_old_home/Library/Application Support/bagent"
old_log="$fixture/old-binary.log"
new_log="$fixture/new-binary.log"
rollback_log="$fixture/rollback-old-binary.log"
post_work_old_log="$fixture/post-work-old-binary.log"
protected_db="$HOME/Library/Application Support/bagent/bagent.db"
mock_basert_pid=""
old_seed_pid=""
rollback_pid=""
new_pid=""
post_work_old_pid=""
restored_pid=""

process_matches() {
    local pid=$1 required=$2 command
    [[ "$pid" =~ ^[0-9]+$ ]] || return 1
    command="$(ps -p "$pid" -o command= 2>/dev/null || true)"
    [[ "$command" == *"$required"* ]]
}

sqlite3_wait() {
    sqlite3 -cmd '.timeout 5000' "$@"
}
[[ "$old_data/bagent.db" != "$protected_db" && "$new_data/bagent.db" != "$protected_db" ]] || {
    echo "A54 FAIL: disposable candidate path resolved to the production database" >&2
    exit 1
}

cleanup() {
    local exit_code=$?
    if (( exit_code != 0 )); then
        for log in "$fixture/mock-basert.log" "$old_log" "$rollback_log" "$new_log" "$post_work_old_log"; do
            if [[ -s "$log" ]]; then
                echo "A54 diagnostic log: $log" >&2
                tail -n 80 "$log" >&2 || true
            fi
        done
    fi
    for pid in "$old_seed_pid" "$rollback_pid" "$new_pid" "$post_work_old_pid" "$restored_pid"; do
        if process_matches "$pid" "/Contents/MacOS/bagentd"; then
            kill -TERM "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
    done
    if process_matches "$mock_basert_pid" "$root/scripts/acceptance/stage8-external-basert-stub.py"; then
        kill -TERM "$mock_basert_pid" 2>/dev/null || true
        wait "$mock_basert_pid" 2>/dev/null || true
    fi
    if [[ -e "$old_checkout/.git" ]]; then
        git worktree remove --force "$old_checkout" >/dev/null 2>&1 || true
    fi
    rm -rf -- "$fixture"
    return "$exit_code"
}
trap cleanup EXIT INT TERM

git worktree add --detach "$old_checkout" "$base"
make -C "$old_checkout/apps/macos" bundle >/dev/null

# The fixed-base production daemon owns the managed 8082 lifecycle. Compile
# acceptance-only daemon variants with the external-runtime adapter instead,
# then use a disposable local HTTP stub. This exercises each signed daemon's
# real startup and schema reader without touching the protected service.
sign_id="$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(Apple Development:[^"]*\)".*/\1/p' | head -n 1)"
[[ -n "$sign_id" ]] || {
    echo "A54 FAIL: Apple Development signing identity unavailable" >&2
    exit 1
}
cargo build --release --manifest-path "$old_checkout/Cargo.toml" -p bagentd \
    --features stage7a-acceptance,stage8-acceptance >/dev/null
cp "$old_checkout/target/release/bagentd" "$old_app/Contents/MacOS/bagentd"
codesign --force --sign "$sign_id" --identifier sk.bagent.app.daemon \
    "$old_app/Contents/MacOS/bagentd" >/dev/null
codesign --force --sign "$sign_id" --identifier sk.bagent.app "$old_app" >/dev/null
codesign --verify --deep --strict "$old_app" || {
    codesign -dvv "$old_app" >&2 || true
    echo "A54 FAIL: signed fixed-base candidate verification failed" >&2
    exit 1
}

cp -R "$candidate" "$new_app"
cargo build --release --manifest-path "$root/Cargo.toml" -p bagentd \
    --features stage7a-acceptance,stage8-acceptance >/dev/null
cp "$root/target/release/bagentd" "$new_app/Contents/MacOS/bagentd"
codesign --force --sign "$sign_id" --identifier sk.bagent.app.daemon \
    "$new_app/Contents/MacOS/bagentd" >/dev/null
codesign --force --sign "$sign_id" --identifier sk.bagent.app "$new_app" >/dev/null
codesign --verify --deep --strict "$new_app" || {
    codesign -dvv "$new_app" >&2 || true
    echo "A54 FAIL: signed current candidate verification failed" >&2
    exit 1
}

mock_port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"
python3 "$root/scripts/acceptance/stage8-external-basert-stub.py" "$mock_port" \
    >"$fixture/mock-basert.log" 2>&1 &
mock_basert_pid=$!
for _ in {1..50}; do
    if curl -fsS "http://127.0.0.1:$mock_port/health" >/dev/null 2>&1; then break; fi
    sleep 0.1
done
kill -0 "$mock_basert_pid" 2>/dev/null || {
    cat "$fixture/mock-basert.log" >&2 || true
    echo "A54 FAIL: disposable BaseRT stub exited" >&2
    exit 1
}
curl -fsS "http://127.0.0.1:$mock_port/health" >/dev/null || {
    cat "$fixture/mock-basert.log" >&2 || true
    echo "A54 FAIL: disposable BaseRT stub did not become healthy" >&2
    exit 1
}

new_hash="$(shasum -a 256 "$new_app/Contents/MacOS/bagentd" | awk '{print $1}')"
old_hash="$(shasum -a 256 "$old_app/Contents/MacOS/bagentd" | awk '{print $1}')"
[[ "$new_hash" != "$old_hash" ]] || {
    echo "A54 FAIL: old and new daemon candidates have the same hash" >&2
    exit 1
}

# First create a V22-era disposable database with the signed old candidate.
# This is the pre-cutover database that the verified backup must preserve.
mkdir -p "$old_home"
set +e
HOME="$old_home" \
BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 \
BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 \
BAGENT_BASERT_BASE_URL="http://127.0.0.1:$mock_port/v1" \
BAGENT_BASERT_API_KEY=stage8-rollback \
    "$old_app/Contents/MacOS/bagentd" >"$old_log" 2>&1 &
old_seed_pid=$!
set -e
old_schema_ready=false
for _ in {1..200}; do
    if [[ -s "$old_data/bagent.db" ]] &&
       sqlite3_wait "$old_data/bagent.db" "SELECT 1 FROM sqlite_master WHERE type='table' AND name='automation_runs'" | rg -qx 1; then
        old_schema_ready=true
        break
    fi
    kill -0 "$old_seed_pid" 2>/dev/null || true
    sleep 0.1
done
[[ "$old_schema_ready" == true ]] || {
    cat "$old_log" >&2
    find "$old_home" -maxdepth 5 -type f -print >&2 || true
    echo "A54 FAIL: fixed-base candidate did not create the pre-cutover schema" >&2
    exit 1
}
if kill -0 "$old_seed_pid" 2>/dev/null; then
    kill -TERM "$old_seed_pid" 2>/dev/null || true
    wait "$old_seed_pid" 2>/dev/null || true
fi
old_seed_pid=""

# Before the first post-cutover Work, restore the verified backup into a new
# disposable home and prove that the signed old candidate can read it.
backup="$fixture/pre-cutover-backup.sqlite"
cp "$old_data/bagent.db" "$backup"
backup_hash="$(shasum -a 256 "$backup" | awk '{print $1}')"
[[ "$backup_hash" == "$(shasum -a 256 "$old_data/bagent.db" | awk '{print $1}')" ]]
old_seed_hash="$backup_hash"
mkdir -p "$rollback_home/Library/Application Support/bagent"
cp "$backup" "$rollback_data/bagent.db"
rollback_hash="$(shasum -a 256 "$rollback_data/bagent.db" | awk '{print $1}')"
[[ "$rollback_hash" == "$backup_hash" ]]
set +e
HOME="$rollback_home" \
BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 \
BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 \
BAGENT_BASERT_BASE_URL="http://127.0.0.1:$mock_port/v1" \
BAGENT_BASERT_API_KEY=stage8-rollback \
    "$old_app/Contents/MacOS/bagentd" >"$rollback_log" 2>&1 &
rollback_pid=$!
set -e
rollback_reader_ready=false
for _ in {1..120}; do
    if [[ -s "$rollback_data/daemon.port" ]] &&
       sqlite3_wait "$rollback_data/bagent.db" "SELECT 1 FROM sqlite_master WHERE type='table' AND name='automation_runs'" | rg -qx 1; then
        rollback_reader_ready=true
        break
    fi
    sleep 0.1
done
[[ "$rollback_reader_ready" == true ]] || {
    cat "$rollback_log" >&2
    echo "A54 FAIL: signed old candidate could not read the verified pre-cutover backup" >&2
    exit 1
}
if kill -0 "$rollback_pid" 2>/dev/null; then
    kill -TERM "$rollback_pid" 2>/dev/null || true
    wait "$rollback_pid" 2>/dev/null || true
fi
rollback_pid=""
rollback_reader_hash="$(shasum -a 256 "$rollback_data/bagent.db" | awk '{print $1}')"
[[ "$backup_hash" == "$(shasum -a 256 "$backup" | awk '{print $1}')" ]]
[[ "$(sqlite3_wait "$rollback_data/bagent.db" 'PRAGMA integrity_check')" == ok ]]

# Cut over the verified pre-cutover backup using the current signed candidate.
# Keep its daemon alive so the first post-cutover Work is admitted through the
# real HTTP boundary rather than a direct SQLite marker mutation.
mkdir -p "$new_home/Library/Application Support/bagent"
cp "$backup" "$new_data/bagent.db"
set +e
HOME="$new_home" \
BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 \
BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 \
BAGENT_BASERT_BASE_URL="http://127.0.0.1:$mock_port/v1" \
BAGENT_BASERT_API_KEY=stage8-rollback \
    "$new_app/Contents/MacOS/bagentd" >"$new_log" 2>&1 &
new_pid=$!
set -e
canonical_ready=false
for _ in {1..240}; do
    if [[ -s "$new_data/bagent.db" ]] &&
       sqlite3_wait "$new_data/bagent.db" "SELECT 1 FROM sqlite_master WHERE type='table' AND name='stage8_cleanup_state'" | rg -qx 1; then
        canonical_ready=true
        break
    fi
    kill -0 "$new_pid" 2>/dev/null || true
    sleep 0.1
done
[[ "$canonical_ready" == true ]] || {
    cat "$new_log" >&2
    find "$new_home" -maxdepth 5 -type f -print >&2 || true
    echo "A54 FAIL: new signed candidate did not migrate the verified pre-cutover backup" >&2
    exit 1
}
sqlite3_wait "$new_data/bagent.db" "SELECT 1 FROM sqlite_master WHERE type='table' AND name='automation_run_records'" | rg -qx 1
if sqlite3_wait "$new_data/bagent.db" "SELECT 1 FROM sqlite_master WHERE type='table' AND name='automation_runs'" | rg -q 1; then
    echo "A54 FAIL: canonical disposable database still exposes automation_runs" >&2
    exit 1
fi

new_port="$(tr -d '[:space:]' < "$new_data/daemon.port")"
new_token="$(tr -d '[:space:]' < "$new_data/daemon.token")"
auth_header="Authorization: Bearer $new_token"
for _ in {1..120}; do
    if curl -fsS -H "$auth_header" "http://127.0.0.1:$new_port/health" >/dev/null 2>&1; then break; fi
    sleep 0.1
done
curl -fsS -H "$auth_header" "http://127.0.0.1:$new_port/health" >/dev/null
automation_payload='{"name":"A54 disposable rollback Work","prompt":"A54 rollback qualification","timezone":"UTC","schedule":{"kind":"once","at":"2099-01-01T00:00:00Z"},"enabled":true}'
automation_response="$(curl -fsS -H "$auth_header" -H 'Content-Type: application/json' -d "$automation_payload" "http://127.0.0.1:$new_port/automations")"
automation_id="$(jq -r .id <<<"$automation_response")"
[[ "$automation_id" != "null" && -n "$automation_id" ]]
run_response="$(curl -fsS -H "$auth_header" -X POST "http://127.0.0.1:$new_port/automations/$automation_id/run-now")"
run_id="$(jq -r .run.id <<<"$run_response")"
[[ "$run_id" != "null" && -n "$run_id" ]]
first_work_seen=false
for _ in {1..120}; do
    first_work_at="$(sqlite3_wait "$new_data/bagent.db" "SELECT COALESCE(first_post_cutover_work_at, '') FROM work_cutover WHERE singleton=1")"
    if [[ -n "$first_work_at" ]] &&
       [[ "$(sqlite3_wait "$new_data/bagent.db" "SELECT COUNT(*) FROM works")" -ge 1 ]]; then
        first_work_seen=true
        break
    fi
    sleep 0.1
done
[[ "$first_work_seen" == true ]] || {
    cat "$new_log" >&2
    echo "A54 FAIL: new signed candidate did not admit a post-cutover Work" >&2
    exit 1
}
if kill -0 "$new_pid" 2>/dev/null; then
    kill -TERM "$new_pid" 2>/dev/null || true
    wait "$new_pid" 2>/dev/null || true
fi
new_pid=""

# After the first post-cutover Work, only the canonical archive is forward
# readable. The old signed candidate must refuse a disposable copy and must
# not rewrite it.
archive="$fixture/forward-archive.sqlite"
cp "$new_data/bagent.db" "$archive"
archive_hash="$(shasum -a 256 "$archive" | awk '{print $1}')"
mkdir -p "$post_work_old_home/Library/Application Support/bagent"
cp "$archive" "$post_work_old_data/bagent.db"
post_work_old_hash="$(shasum -a 256 "$post_work_old_data/bagent.db" | awk '{print $1}')"
set +e
HOME="$post_work_old_home" \
BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 \
BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 \
BAGENT_BASERT_BASE_URL="http://127.0.0.1:$mock_port/v1" \
BAGENT_BASERT_API_KEY=stage8-rollback \
    "$old_app/Contents/MacOS/bagentd" >"$post_work_old_log" 2>&1 &
post_work_old_pid=$!
set -e
old_exited=false
for _ in {1..120}; do
    if ! kill -0 "$post_work_old_pid" 2>/dev/null; then
        old_exited=true
        break
    fi
    sleep 0.1
done
if [[ "$old_exited" == false ]]; then
    kill -TERM "$post_work_old_pid" 2>/dev/null || true
    wait "$post_work_old_pid" 2>/dev/null || true
    echo "A54 FAIL: fixed-base binary stayed alive on the post-Work canonical database" >&2
    exit 1
fi
old_status=0
wait "$post_work_old_pid" 2>/dev/null || old_status=$?
post_work_old_pid=""
[[ "$old_status" -ne 0 ]] || {
    cat "$post_work_old_log" >&2
    echo "A54 FAIL: fixed-base binary returned success on the post-Work canonical database" >&2
    exit 1
}
if ! rg -qi 'automation_runs|no such table|finalize|migration' "$post_work_old_log"; then
    cat "$post_work_old_log" >&2
    echo "A54 FAIL: fixed-base refusal did not identify the post-cutover boundary" >&2
    exit 1
fi
[[ "$post_work_old_hash" == "$(shasum -a 256 "$post_work_old_data/bagent.db" | awk '{print $1}')" ]]
[[ "$archive_hash" == "$(shasum -a 256 "$archive" | awk '{print $1}')" ]]

# Downgrade after cutover is archive-and-restore: restore the verified old
# backup, then prove the signed old candidate can read that restored copy.
rm -f "$rollback_data/bagent.db"
cp "$backup" "$rollback_data/bagent.db"
restored_hash="$(shasum -a 256 "$rollback_data/bagent.db" | awk '{print $1}')"
[[ "$restored_hash" == "$backup_hash" ]]
set +e
HOME="$rollback_home" \
BAGENT_STAGE7A_ACCEPTANCE_FIXTURE=1 \
BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 \
BAGENT_BASERT_BASE_URL="http://127.0.0.1:$mock_port/v1" \
BAGENT_BASERT_API_KEY=stage8-rollback \
    "$old_app/Contents/MacOS/bagentd" >"$rollback_log" 2>&1 &
restored_pid=$!
set -e
restored_reader_ready=false
for _ in {1..120}; do
    if [[ -s "$rollback_data/daemon.port" ]] &&
       sqlite3_wait "$rollback_data/bagent.db" "SELECT 1 FROM sqlite_master WHERE type='table' AND name='automation_runs'" | rg -qx 1; then
        restored_reader_ready=true
        break
    fi
    sleep 0.1
done
[[ "$restored_reader_ready" == true ]] || {
    cat "$rollback_log" >&2
    echo "A54 FAIL: archive-and-restore did not produce an old-readable database" >&2
    exit 1
}
if kill -0 "$restored_pid" 2>/dev/null; then
    kill -TERM "$restored_pid" 2>/dev/null || true
    wait "$restored_pid" 2>/dev/null || true
fi
restored_pid=""
restored_reader_hash="$(shasum -a 256 "$rollback_data/bagent.db" | awk '{print $1}')"
[[ "$backup_hash" == "$(shasum -a 256 "$backup" | awk '{print $1}')" ]]
[[ "$(sqlite3_wait "$rollback_data/bagent.db" 'PRAGMA integrity_check')" == ok ]]

# The integration gate proves the first-Work boundary, old-reader refusal,
# archive-and-restore downgrade, and both disposable archive hashes.
cargo test -p bagentd --test persistence_migration cutover_boundary -- --exact --nocapture \
    | tee "$fixture/cutover-boundary.log"
rg -q 'A54 pre-Work backup SHA-256:' "$fixture/cutover-boundary.log"
rg -q 'A54 post-Work canonical archive SHA-256:' "$fixture/cutover-boundary.log"

protected_8080_after="$(lsof -nP -tiTCP:8080 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
protected_8082_after="$(lsof -nP -tiTCP:8082 -sTCP:LISTEN 2>/dev/null | sort | tr '\n' ',' || true)"
[[ "$protected_8080_before" == "$protected_8080_after" ]] || {
    echo "A54 FAIL: protected port 8080 listener changed" >&2
    exit 1
}
[[ "$protected_8082_before" == "$protected_8082_after" ]] || {
    echo "A54 FAIL: protected port 8082 listener changed" >&2
    exit 1
}

echo "A54 rollback qualification: PASS (macOS $product_version; signed old/new daemon hashes differ; pre-cutover old DB hash=$old_seed_hash; verified backup=$backup_hash; old reader copy hash=$rollback_reader_hash; signed new migration and first post-cutover Work executed; post-Work canonical archive=$archive_hash; old binary refused only the post-Work canonical DB; archive-and-restore source hash=$restored_hash; restored old-reader copy hash=$restored_reader_hash; protected ports unchanged; all disposable fixtures cleaned)"

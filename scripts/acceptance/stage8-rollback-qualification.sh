#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
candidate="${1:-$root/apps/macos/bagent.app}"
base="${STAGE8_FIXED_BASE:-45c26b1c1d3bd482b144525723a9c71a1fe57ced}"
product_version="$(sw_vers -productVersion)"
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
new_home="$fixture/new-home"
old_home="$fixture/old-home"
new_data="$new_home/Library/Application Support/bagent"
old_log="$fixture/old-binary.log"
new_log="$fixture/new-binary.log"
protected_db="$HOME/Library/Application Support/bagent/bagent.db"
[[ "$new_data/bagent.db" != "$protected_db" ]] || {
    echo "A54 FAIL: disposable candidate path resolved to the production database" >&2
    exit 1
}

cleanup() {
    if [[ -e "$old_checkout/.git" ]]; then
        git worktree remove --force "$old_checkout" >/dev/null 2>&1 || true
    fi
    rm -rf -- "$fixture"
}
trap cleanup EXIT INT TERM

git worktree add --detach "$old_checkout" "$base"
make -C "$old_checkout/apps/macos" bundle >/dev/null
codesign --verify --deep --strict "$old_app"

new_hash="$(shasum -a 256 "$candidate/Contents/MacOS/bagentd" | awk '{print $1}')"
old_hash="$(shasum -a 256 "$old_app/Contents/MacOS/bagentd" | awk '{print $1}')"
[[ "$new_hash" != "$old_hash" ]] || {
    echo "A54 FAIL: old and new daemon candidates have the same hash" >&2
    exit 1
}

# A clean disposable install is converted by the new signed daemon. Its
# runtime is deliberately pointed at an unused loopback port, so the process
# exits after it has committed the schema boundary without touching any live
# BaseRT service.
mkdir -p "$new_home"
set +e
HOME="$new_home" \
BAGENT_BASERT_BASE_URL="http://127.0.0.1:9/v1" \
BAGENT_BASERT_API_KEY=stage8-rollback \
    "$candidate/Contents/MacOS/bagentd" >"$new_log" 2>&1 &
new_pid=$!
set -e
canonical_ready=false
for _ in {1..200}; do
    if [[ -s "$new_data/bagent.db" ]] &&
       sqlite3 "$new_data/bagent.db" "SELECT 1 FROM sqlite_master WHERE type='table' AND name='stage8_cleanup_state'" | rg -qx 1; then
        canonical_ready=true
        break
    fi
    kill -0 "$new_pid" 2>/dev/null || true
    sleep 0.1
done
[[ "$canonical_ready" == true ]] || {
    cat "$new_log" >&2
    find "$new_home" -maxdepth 5 -type f -print >&2 || true
    echo "A54 FAIL: new candidate did not commit canonical schema" >&2
    exit 1
}
if kill -0 "$new_pid" 2>/dev/null; then
    kill -TERM "$new_pid" 2>/dev/null || true
    wait "$new_pid" 2>/dev/null || true
fi

sqlite3 "$new_data/bagent.db" "SELECT 1 FROM sqlite_master WHERE type='table' AND name='automation_run_records'" | rg -qx 1
if sqlite3 "$new_data/bagent.db" "SELECT 1 FROM sqlite_master WHERE type='table' AND name='automation_runs'" | rg -q 1; then
    echo "A54 FAIL: canonical disposable database still exposes automation_runs" >&2
    exit 1
fi
backup="$fixture/pre-cutover-backup.sqlite"
cp "$new_data/bagent.db" "$backup"
backup_hash="$(shasum -a 256 "$backup" | awk '{print $1}')"
[[ "$backup_hash" == "$(shasum -a 256 "$new_data/bagent.db" | awk '{print $1}')" ]]

# The integration gate proves the first-Work boundary, old-reader refusal,
# archive-and-restore downgrade, and both disposable archive hashes.
cargo test -p bagentd --test persistence_migration cutover_boundary -- --exact --nocapture \
    | tee "$fixture/cutover-boundary.log"
rg -q 'A54 pre-Work backup SHA-256:' "$fixture/cutover-boundary.log"
rg -q 'A54 post-Work canonical archive SHA-256:' "$fixture/cutover-boundary.log"

# Exercise the actual fixed-base daemon against the committed canonical DB.
# Its pre-Stage-8 migration path must fail closed when automation_runs is gone;
# the old binary is never allowed to read or rewrite the new database.
mkdir -p "$old_home/Library/Application Support/bagent"
cp "$new_data/bagent.db" "$old_home/Library/Application Support/bagent/bagent.db"
old_db_hash="$(shasum -a 256 "$old_home/Library/Application Support/bagent/bagent.db" | awk '{print $1}')"
set +e
HOME="$old_home" \
BAGENT_BASERT_BASE_URL="http://127.0.0.1:9/v1" \
BAGENT_BASERT_API_KEY=stage8-rollback \
    "$old_app/Contents/MacOS/bagentd" >"$old_log" 2>&1 &
old_pid=$!
set -e
old_exited=false
for _ in {1..120}; do
    if ! kill -0 "$old_pid" 2>/dev/null; then
        old_exited=true
        break
    fi
    sleep 0.1
done
if [[ "$old_exited" == false ]]; then
    kill -TERM "$old_pid" 2>/dev/null || true
    wait "$old_pid" 2>/dev/null || true
    echo "A54 FAIL: fixed-base binary stayed alive on the canonical database" >&2
    exit 1
fi
old_status=0
wait "$old_pid" 2>/dev/null || old_status=$?
[[ "$old_status" -ne 0 ]] || {
    cat "$old_log" >&2
    echo "A54 FAIL: fixed-base binary returned success on the canonical database" >&2
    exit 1
}
if ! rg -qi 'automation_runs|no such table|finalize|migration' "$old_log"; then
    cat "$old_log" >&2
    echo "A54 FAIL: fixed-base refusal did not identify the migration/database boundary" >&2
    exit 1
fi
[[ "$old_db_hash" == "$(shasum -a 256 "$old_home/Library/Application Support/bagent/bagent.db" | awk '{print $1}')" ]] || {
    echo "A54 FAIL: fixed-base binary rewrote the canonical database" >&2
    exit 1
}

echo "A54 rollback qualification: PASS (macOS $product_version; fixed-base and current signed daemon hashes differ; disposable canonical DB and verified backup hash=$backup_hash; fixed-base binary refused canonical DB before any Work; integration archive-and-restore hashes recorded; cleanup complete)"

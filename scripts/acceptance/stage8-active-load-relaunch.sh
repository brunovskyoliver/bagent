#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
candidate="${1:-$root/apps/macos/bagent.app}"
product_version="$(sw_vers -productVersion)"
[[ "${product_version%%.*}" == "26" ]] || {
    echo "A58 BLOCKED: signed relaunch qualification is macOS 26 only (found $product_version)" >&2
    exit 1
}
codesign --verify --deep --strict "$candidate"

for test_name in fairness_foreground automation_capacity_two approval_restart_capacity cancellation_races; do
    cargo test -p bagentd --test work_concurrency "$test_name" -- --exact
done
cargo test -p bagentd --test model_runtime poison_changed_pid -- --exact
cargo test -p bagentd --test model_runtime port_isolation -- --exact

# This is the signed A49 fixture: it starts two disposable runtime components,
# a real daemon and the candidate UI, freezes only the disposable BaseRT, and
# checks the old/new UI fence, cursor/revision convergence, lease/capacity
# preservation, timeout rollback, and the protected 8080 sentinel.
"$root/scripts/acceptance/stage7c-production-ui-relaunch.sh" "$candidate"

echo "A58 active-load relaunch: PASS (macOS $product_version; A18-A21 nonzero; changed-PID poison and 8080 isolation nonzero; signed A49 UI-only relaunch with disposable daemon/BaseRT and protected port sentinel)"

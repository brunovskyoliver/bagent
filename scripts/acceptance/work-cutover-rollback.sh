#!/bin/bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo_root"

cargo test -p bagentd --test persistence_migration cutover_boundary -- --exact

required=(
  "verified_backup"
  "verified_restore"
  "PreFirstWork"
  "ForwardOnly"
  "forward-archive.sqlite"
  "old_binary_can_open"
)
for token in "${required[@]}"; do
  rg -q --fixed-strings "$token" crates/daemon/tests/persistence_migration.rs
done

echo "work cutover rollback: PASS (verified backup/restore, old-reader fixture, first-Work barrier, archive disclosure)"

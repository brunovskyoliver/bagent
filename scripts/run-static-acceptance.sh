#!/usr/bin/env bash
set -uo pipefail

if [[ "$#" -ne 3 ]]; then
  printf 'usage: %s NAME EXPECTED_ASSERTION_COUNT SCRIPT\n' "$0" >&2
  exit 2
fi

name="$1"
assertion_count="$2"
shift 2
script="$1"

if [[ ! "$assertion_count" =~ ^[1-9][0-9]*$ ]]; then
  printf 'STATIC_ACCEPTANCE name=%s verdict=BLOCKED assertions=0 reason=invalid_assertion_count\n' "$name"
  exit 2
fi

measured_assertions="$({ grep -Ec '^(require|reject|reject_in) ' "$script" || true; } | tail -n 1)"
marked_assertions="$(grep -Ec '^# acceptance-assertion:' "$script" || true)"
measured_assertions=$((measured_assertions + marked_assertions))
if [[ "$measured_assertions" -eq 0 || "$measured_assertions" -ne "$assertion_count" ]]; then
  printf 'STATIC_ACCEPTANCE name=%s verdict=BLOCKED assertions=%s expected_assertions=%s reason=assertion_count_mismatch\n' \
    "$name" "$measured_assertions" "$assertion_count"
  exit 2
fi

output_file="$(mktemp "${TMPDIR:-/tmp}/bagent-static-acceptance.XXXXXX")"
trap 'rm -f "$output_file"' EXIT

bash "$script" >"$output_file" 2>&1
command_status=$?
cat "$output_file"

if [[ "$command_status" -eq 0 ]]; then
  printf 'STATIC_ACCEPTANCE name=%s verdict=PASS assertions=%s exit_status=0\n' \
    "$name" "$measured_assertions"
  exit 0
fi

printf 'STATIC_ACCEPTANCE name=%s verdict=FAIL assertions=%s exit_status=%s\n' \
  "$name" "$measured_assertions" "$command_status"
exit 1

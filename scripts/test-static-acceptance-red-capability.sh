#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
runner="$repo_root/scripts/run-static-acceptance.sh"
output_file="$(mktemp "${TMPDIR:-/tmp}/bagent-static-red-capability.XXXXXX")"
zero_script="$(mktemp "${TMPDIR:-/tmp}/bagent-static-zero.XXXXXX")"
red_script="$(mktemp "${TMPDIR:-/tmp}/bagent-static-red.XXXXXX")"
trap 'rm -f "$output_file" "$zero_script" "$red_script"' EXIT

printf 'exit 0\n' >"$zero_script"
if "$runner" synthetic-zero 1 "$zero_script" >"$output_file" 2>&1; then
  printf 'FAIL: static acceptance runner accepted a zero-assertion check\n'
  exit 1
fi
grep -Fq 'verdict=BLOCKED assertions=0 expected_assertions=1' "$output_file"

printf '# acceptance-assertion: synthetic red\nexit 7\n' >"$red_script"
if "$runner" synthetic-red 1 "$red_script" >"$output_file" 2>&1; then
  printf 'FAIL: static acceptance runner promoted a failing check\n'
  exit 1
fi

grep -Fq 'verdict=FAIL assertions=1 exit_status=7' "$output_file"
printf 'PASS: static acceptance runner rejects zero counts and preserves a failing verdict (1 assertion)\n'

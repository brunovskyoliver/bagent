#!/usr/bin/env bash
set -euo pipefail

fail=0

reject() {
  local file="$1"
  local pattern="$2"
  local message="$3"
  if grep -Fq "$pattern" "$file"; then
    printf 'FAIL: %s\n' "$message"
    fail=1
  fi
}

require() {
  local file="$1"
  local pattern="$2"
  local message="$3"
  if ! grep -Fq "$pattern" "$file"; then
    printf 'FAIL: %s\n' "$message"
    fail=1
  fi
}

require crates/connectors/apple_mail/src/lib.rs "set rcpt to" "AppleScript mail search does not collect recipient"
require crates/connectors/apple_mail/src/lib.rs "parse_applescript_mail_record" "AppleScript mail records are not parsed at a testable seam"
reject crates/connectors/apple_mail/src/lib.rs "value.trim().parse::<f64>()" "AppleScript timestamp parser is still locale-fragile"
require crates/connectors/apple_mail/src/lib.rs "replace(',', \".\")" "AppleScript timestamp parser does not accept comma decimals"
require crates/daemon/src/main.rs '**Komu:** {komu}' "AppleScript mail output does not render actual recipient"
require crates/daemon/src/main.rs 'return "neznáme".to_string();' "invalid mail dates can still render as Unix epoch"

# acceptance-assertion: parsed records preserve recipients
if sed -n '/fn parse_applescript_mail_record/,/^}/p' crates/connectors/apple_mail/src/lib.rs | grep -Fq "recipient: None"; then
  printf 'FAIL: AppleScript parsed record still drops recipient\n'
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

printf 'PASS: AppleScript mail rendering regressions covered\n'

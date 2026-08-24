#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
runner="$repo_root/scripts/run-static-acceptance.sh"
fail=0

run_check() {
  "$runner" "$1" "$2" "$repo_root/$3" || fail=1
}

run_check inline-input-focus 6 scripts/check-inline-input-focus-retention.sh
run_check input-overflow-non-promotion 10 scripts/check-no-input-overflow-promotion.sh
run_check thinking-output-dot-layer 22 scripts/check-notch-thinking-output-dot-layer.sh
run_check fullscreen-notch-visibility 6 scripts/check-fullscreen-notch-visibility.sh
run_check non-notch-inline-pill 6 scripts/check-non-notch-inline-pill.sh
run_check notch-output-scroll-stability 17 scripts/check-notch-output-scroll-stability.sh
run_check notch-output-layout 19 scripts/check-notch-output-regressions.sh
run_check mail-rendering 7 scripts/check-mail-rendering-regressions.sh

exit "$fail"

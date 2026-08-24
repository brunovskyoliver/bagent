#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
root="${1:-$repo_root}"
settings="$root/apps/macos/Sources/bagent/NotchSettingsContent.swift"
route="$root/apps/macos/Sources/bagent/CompassRail.swift"
view_model="$root/apps/macos/Sources/bagent/ChatViewModel.swift"
chat_view="$root/apps/macos/Sources/bagent/ChatView.swift"
projection="$root/apps/macos/Sources/bagent/NotchProjection.swift"

surface_count=0
assertion_count=0
failure_count=0

fail() {
  printf 'FAIL: %s\n' "$1" >&2
  failure_count=$((failure_count + 1))
}

require_surface() {
  local file="$1"
  local label="$2"
  if [[ ! -s "$file" ]]; then
    fail "$label is missing or empty"
    return
  fi
  local lines
  lines="$(wc -l < "$file" | tr -d ' ')"
  if [[ "$lines" -le 0 ]]; then
    fail "$label has zero production lines"
  else
    surface_count=$((surface_count + 1))
    printf 'surface %s: %s lines\n' "$label" "$lines"
  fi
}

reject_pattern() {
  local file="$1"
  local pattern="$2"
  local label="$3"
  assertion_count=$((assertion_count + 1))
  if rg -n -i --glob '*.swift' "$pattern" "$file" >/dev/null; then
    fail "$label"
  else
    printf 'assertion %d: %s\n' "$assertion_count" "$label"
  fi
}

require_pattern() {
  local file="$1"
  local pattern="$2"
  local label="$3"
  assertion_count=$((assertion_count + 1))
  if rg -n "$pattern" "$file" >/dev/null; then
    printf 'assertion %d: %s\n' "$assertion_count" "$label"
  else
    fail "$label"
  fi
}

require_surface "$settings" "NotchSettingsContent"
require_surface "$route" "CompassRail"
require_surface "$view_model" "ChatViewModel"
require_surface "$chat_view" "ChatView"
require_surface "$projection" "NotchProjection"

require_pattern "$route" 'case general|case modelRuntime|case integrations|case privacyAndPermissions' \
  'Compass Rail declares the four accepted peers'
reject_pattern "$route" 'case setup\b|case permissions\b|case connectors\b|case model\b' \
  'Compass Rail has no fifth or legacy peer'
reject_pattern "$view_model" 'NotchSettingsPage|notchSettingsPage|setupBridgeHeight|\.setup' \
  'production mode and geometry have no Setup route'
reject_pattern "$settings" 'ScrollView|LazyV|TextEditor' \
  'settings presentation has no scrolling or raw editor'
reject_pattern "$settings" 'rules\.yaml|Ollama|availableModels|selectedClassifierModel|provider(Name|URL|Model|Config)|fallback' \
  'settings presentation has no legacy provider/fallback controls'
reject_pattern "$settings" 'settingsGroup|rulesGroup|odooGroup|codexGroup|whatsappGroup|legacyConnector' \
  'legacy connector/setup layout is absent'
reject_pattern "$settings" 'rawError|providerError|credential|connectorIdentifier' \
  'settings presentation does not expose raw provider data or secrets'
reject_pattern "$view_model" 'NotchSettingsPage|notchSettingsPage' \
  'ChatViewModel has no duplicate legacy selection state'
require_pattern "$view_model" '@Published var compassRailRoute: CompassRailRoute' \
  'ChatViewModel has one typed writable settings route'
require_pattern "$chat_view" 'NotchWrapMetrics\.settingsWingWidth|NotchWrapMetrics\.settingsBridgeHeight' \
  'ChatView uses ordinary settings geometry'
require_pattern "$chat_view" 'if mode == \.settings \{ return base \}' \
  'settings geometry ignores projection pill expansion'
reject_pattern "$chat_view" 'setupBridgeHeight|notchSettingsPage|\.setup' \
  'ChatView has no special Setup geometry or route'

if [[ "$surface_count" -eq 0 ]]; then
  fail 'zero production surfaces measured'
fi
if [[ "$assertion_count" -eq 0 ]]; then
  fail 'zero assertions executed'
fi
if [[ "$failure_count" -ne 0 ]]; then
  printf 'settings authority: %d surfaces, %d assertions, %d failures\n' "$surface_count" "$assertion_count" "$failure_count" >&2
  exit 1
fi

# Red capability: a disposable copy with a Setup route must fail the same gate.
fixture="$(mktemp -d "${TMPDIR:-/tmp}/bagent-settings-authority.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/apps/macos/Sources/bagent"
cp "$settings" "$fixture/apps/macos/Sources/bagent/NotchSettingsContent.swift"
cp "$route" "$fixture/apps/macos/Sources/bagent/CompassRail.swift"
cp "$view_model" "$fixture/apps/macos/Sources/bagent/ChatViewModel.swift"
cp "$chat_view" "$fixture/apps/macos/Sources/bagent/ChatView.swift"
cp "$projection" "$fixture/apps/macos/Sources/bagent/NotchProjection.swift"
printf '\n' >> "$fixture/apps/macos/Sources/bagent/NotchSettingsContent.swift"
printf 'case setup\n' >> "$fixture/apps/macos/Sources/bagent/CompassRail.swift"
if bash "$0" "$fixture" >/dev/null 2>&1; then
  fail 'red capability did not reject a seeded Setup route'
else
  printf 'red capability: seeded Setup route rejected\n'
fi

printf 'settings authority: %d surfaces, %d assertions, 0 failures\n' "$surface_count" "$assertion_count"

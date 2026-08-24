#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

scan_tree() {
    local root="$1"
    local failures=0

    if rg -n --glob '*.swift' 'currentChatIdentity\s*=.*UUID|current-chat-.*UUID' "$root/apps/macos/Sources" >/dev/null 2>&1; then
        echo "FAIL: Swift generates an authoritative Current Chat identity"
        failures=$((failures + 1))
    fi
    if rg -n --glob '*.swift' 'bagent\.session_id|UserDefaults[^\n]*(current.?chat|session.?id)' "$root/apps/macos/Sources" >/dev/null 2>&1; then
        echo "FAIL: UserDefaults retains chat or session identity authority"
        failures=$((failures + 1))
    fi
    if rg -n --glob '*.swift' 'func clear\(\)' "$root/apps/macos/Sources" >/dev/null 2>&1; then
        echo "FAIL: a local-only clear entry point remains"
        failures=$((failures + 1))
    fi
    if rg -n --glob '*.rs' 'automation_current_chats' "$root/crates/daemon/src" >/dev/null 2>&1; then
        if ! rg -n --glob '*.rs' 'DROP TABLE IF EXISTS automation_current_chats' "$root/crates/daemon/src" >/dev/null 2>&1; then
            echo "FAIL: writable legacy Current Chat identity path remains"
            failures=$((failures + 1))
        fi
    fi
    if rg -n --glob '*.swift' 'diacriticInsensitive|folding\(' "$root/apps/macos/Sources/bagent/SlashCommandRegistry.swift" >/dev/null 2>&1; then
        echo "FAIL: fuzzy or diacritic-folded Slash Command matching remains"
        failures=$((failures + 1))
    fi
    if rg -U -n --glob '*.swift' 'exactMatch\([^}]{0,500}trimmingCharacters' "$root/apps/macos/Sources" >/dev/null 2>&1; then
        echo "FAIL: Slash Command execution trims the raw draft"
        failures=$((failures + 1))
    fi
    if rg -U -n --glob '*.swift' 'completeSlashSuggestion[^}]{0,500}(send\(|execute\()' "$root/apps/macos/Sources" >/dev/null 2>&1; then
        echo "FAIL: suggestion completion can execute"
        failures=$((failures + 1))
    fi

    local registries
    registries="$(rg -n --glob '*.swift' 'static let all: \[SlashCommand\]' "$root/apps/macos/Sources" 2>/dev/null | wc -l | tr -d ' ')"
    if [[ "$registries" != "1" ]]; then
        echo "FAIL: expected one typed Slash Command registry, measured $registries"
        failures=$((failures + 1))
    fi

    return "$failures"
}

red_fixture="$(mktemp -d "${TMPDIR:-/tmp}/bagent-stage7a-red.XXXXXX")"
trap 'rm -rf -- "$red_fixture"' EXIT
mkdir -p "$red_fixture/apps/macos/Sources/bagent" "$red_fixture/crates/daemon/src"
printf '%s\n' \
    'struct SlashCommand {}' \
    'enum BadRegistry { static let all: [SlashCommand] = [] }' \
    'enum DuplicateRegistry { static let all: [SlashCommand] = [] }' \
    'let currentChatIdentity = "current-chat-" + UUID().uuidString' \
    'let old = UserDefaults.standard.string(forKey: "bagent.session_id")' \
    'func clear() {}' \
    'func fuzzy() { _ = "/clear".folding(options: .diacriticInsensitive, locale: nil) }' \
    'func exactMatch(_ raw: String) { _ = raw.trimmingCharacters(in: .whitespaces) }' \
    'func completeSlashSuggestion() { send() }' \
    > "$red_fixture/apps/macos/Sources/bagent/SlashCommandRegistry.swift"
printf '%s\n' 'const LEGACY: &str = "automation_current_chats";' \
    > "$red_fixture/crates/daemon/src/bad.rs"

set +e
red_output="$(scan_tree "$red_fixture" 2>&1)"
red_status=$?
set -e
if [[ "$red_status" -eq 0 ]]; then
    echo "FAIL: red-capability fixture did not trip the authority scanner"
    exit 1
fi
red_failures="$(printf '%s\n' "$red_output" | rg -c '^FAIL:')"
if [[ "$red_failures" != "8" ]]; then
    echo "FAIL: red-capability fixture measured $red_failures forbidden surfaces, expected 8"
    exit 1
fi
for expected in \
    'Swift generates an authoritative Current Chat identity' \
    'UserDefaults retains chat or session identity authority' \
    'a local-only clear entry point remains' \
    'writable legacy Current Chat identity path remains' \
    'fuzzy or diacritic-folded Slash Command matching remains' \
    'Slash Command execution trims the raw draft' \
    'suggestion completion can execute' \
    'expected one typed Slash Command registry'; do
    if ! printf '%s\n' "$red_output" | rg -F "$expected" >/dev/null; then
        echo "FAIL: red-capability fixture missed detector: $expected"
        exit 1
    fi
done

scan_tree "$repo_root"

swift_surfaces="$(rg -l 'currentChat\(\)|clearCurrentChat|saveCurrentChatDraft' "$repo_root/apps/macos/Sources/bagent" --glob '*.swift' | wc -l | tr -d ' ')"
rust_surfaces="$(rg -l 'current_chat_authority|clear_current_chat|recover_after_daemon_restart' "$repo_root/crates/daemon/src" "$repo_root/crates/daemon/migrations/V26__durable_current_chat.sql" --glob '*.rs' --glob '*.sql' | wc -l | tr -d ' ')"
command_entries="$(rg -n 'command: "/(settings|automations|clear)"' "$repo_root/apps/macos/Sources/bagent/SlashCommandRegistry.swift" | wc -l | tr -d ' ')"

if [[ "$swift_surfaces" -eq 0 || "$rust_surfaces" -eq 0 || "$command_entries" -eq 0 ]]; then
    echo "FAIL: production surface measurement was zero"
    exit 1
fi
if ! rg -n 'DROP TABLE IF EXISTS automation_current_chats' "$repo_root/crates/daemon/migrations/V26__durable_current_chat.sql" >/dev/null; then
    echo "FAIL: V22 does not retire the writable legacy identity table"
    exit 1
fi

echo "A42 PASS: Swift surfaces=$swift_surfaces Rust surfaces=$rust_surfaces command entries=$command_entries red failures=$red_failures"

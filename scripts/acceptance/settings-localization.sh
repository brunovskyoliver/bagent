#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
catalog="$root/apps/macos/Sources/bagent/Resources/Localizable.xcstrings"

jq -e '.sourceLanguage == "en" and (.strings | type == "object")' "$catalog" >/dev/null
for key in \
    settings.area.general \
    settings.area.modelRuntime \
    settings.area.integrations \
    settings.area.privacy \
    settings.child.whatsapp \
    settings.child.odoo \
    settings.child.codex \
    settings.child.fullDiskAccess \
    settings.child.screenRecording \
    settings.child.accessibility \
    settings.child.rules; do
    jq -e --arg key "$key" '.strings[$key].localizations.en.stringUnit.value != null and .strings[$key].localizations.ja.stringUnit.value != null' "$catalog" >/dev/null
done

for key in \
    "API key" "Apple Mail" "Apple Notes" "Automatic from PATH" \
    "Binary" "Codex configuration" "Connect" "Daemon-owned" \
    "Full Disk Access" "Hold the right Command key for recent clips" \
    "None" "Not selected" "Odoo configuration" "Open" \
    "Opens Privacy and Permissions" "Rules and approval policy" \
    "Selected" "Status" "Test" "Testing…" "URL" "User" \
    "Validation" "WhatsApp configuration" "cmux notifications" \
    "Compass Rail" "In progress" "Back to General" \
    "Back to Model and Runtime" "Back to Integrations" \
    "Back to Privacy and Permissions" "https://company.odoo.com" \
    "company" "user@example.com" "seconds" "WhatsApp" "Odoo" "Codex" \
    "Accessibility" "Screen Recording" "Active — unloading prevented"; do
    jq -e --arg key "$key" '.strings[$key].localizations.en.stringUnit.value != null and .strings[$key].localizations.ja.stringUnit.value != null' "$catalog" >/dev/null
done

keys=$(jq '.strings | length' "$catalog")
test "$keys" -gt 0
echo "settings localization: valid JSON catalog, $keys keys, required Compass Rail labels complete in en and ja"

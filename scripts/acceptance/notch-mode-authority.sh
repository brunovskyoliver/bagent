#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
sources="$root/apps/macos/Sources/bagent"

forbidden='@Published[[:space:]]+var[[:space:]]+(isThinking|isExpanded|chatSurfaceMode|toolStatus)|@Published[[:space:]]+var[[:space:]]+notchInteractionMode|globalEvents\(|authedRequest\("/events"'

if rg -n "$forbidden" "$sources"; then
    echo "FAIL: writable parallel notch authority or legacy event consumer remains" >&2
    exit 1
fi

rg -q 'var[[:space:]]+notchPresentation.*notchEventConsumer\.presentation' "$sources/ChatViewModel.swift"
rg -q 'static func reduce' "$sources/NotchProjection.swift"
rg -q 'final class NotchEventConsumer' "$sources/NotchEventConsumer.swift"

echo "PASS: NotchInteractionMode is stored only in the reducer presentation; no writable parallel mode or legacy event consumer remains"

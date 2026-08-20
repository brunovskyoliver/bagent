#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="${0:A:h}"
REPO_ROOT="${SCRIPT_DIR:h}"
MACOS_DIR="${REPO_ROOT}/apps/macos"
HARNESS_APP="${MACOS_DIR}/Phase0Harness.app"
HARNESS_EXECUTABLE="${HARNESS_APP}/Contents/MacOS/bagent-browser-harness"

cd "$MACOS_DIR"
swift build -c release --product bagent-browser-harness
mkdir -p "$HARNESS_APP/Contents/MacOS"
cp .build/release/bagent-browser-harness "$HARNESS_EXECUTABLE"
cp Phase0Harness-Info.plist "$HARNESS_APP/Contents/Info.plist"
codesign --force --deep --sign - "$HARNESS_APP"
codesign --verify --deep --strict --verbose=2 "$HARNESS_APP"

python3 -m http.server 8765 --bind 127.0.0.1 --directory BrowserFixtures >/tmp/bagent-phase0-http.log 2>&1 &
HTTP_PID=$!
trap 'kill "$HTTP_PID" 2>/dev/null || true' EXIT

"$HARNESS_EXECUTABLE" all
"$HARNESS_EXECUTABLE" cookie-write
"$HARNESS_EXECUTABLE" cookie-read

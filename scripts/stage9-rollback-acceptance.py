#!/usr/bin/env python3
"""Verify signed Stage 9 rollback routing without exposing private payloads."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import plistlib
import sqlite3
import subprocess
import time
import urllib.error
import urllib.request


PROTECTED_TABLES = (
    "approvals",
    "automations",
    "automation_runs",
    "connectors",
    "mail_attachments",
    "mail_cache",
    "pending_approvals",
)


def run(*arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(arguments, check=True, text=True, capture_output=True)


def scalar(path: pathlib.Path) -> str:
    return path.read_text().strip()


def wait_for_daemon(data_dir: pathlib.Path, timeout: float = 45.0) -> tuple[str, str]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            port = scalar(data_dir / "daemon.port")
            token = scalar(data_dir / "daemon.token")
            request = urllib.request.Request(
                f"http://127.0.0.1:{port}/health",
                headers={"Authorization": f"Bearer {token}"},
            )
            with urllib.request.urlopen(request, timeout=2) as response:
                if response.status == 200:
                    return port, token
        except (OSError, urllib.error.URLError):
            time.sleep(0.25)
    raise RuntimeError("signed rollback daemon did not become healthy")


def normalize(value: object) -> object:
    if isinstance(value, bytes):
        return {"blob_sha256": hashlib.sha256(value).hexdigest(), "bytes": len(value)}
    return value


def protected_snapshot(database: pathlib.Path) -> dict[str, dict[str, object]]:
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    snapshot: dict[str, dict[str, object]] = {}
    try:
        existing = {
            row[0]
            for row in connection.execute(
                "SELECT name FROM sqlite_master WHERE type='table'"
            )
        }
        for table in PROTECTED_TABLES:
            if table not in existing:
                snapshot[table] = {"present": False}
                continue
            columns = [row[1] for row in connection.execute(f'PRAGMA table_info("{table}")')]
            selected = columns
            if table == "connectors":
                # Background Mail observation may update last_sync_at. Rollback
                # must preserve connector identity, configuration, and enablement.
                selected = [column for column in columns if column != "last_sync_at"]
            quoted = ", ".join(f'"{column}"' for column in selected)
            rows = [
                [normalize(value) for value in row]
                for row in connection.execute(f'SELECT {quoted} FROM "{table}"')
            ]
            rows.sort(key=lambda row: json.dumps(row, sort_keys=True, ensure_ascii=False))
            canonical = json.dumps(
                {"columns": selected, "rows": rows},
                sort_keys=True,
                ensure_ascii=False,
                separators=(",", ":"),
            ).encode()
            snapshot[table] = {
                "present": True,
                "rows": len(rows),
                "sha256": hashlib.sha256(canonical).hexdigest(),
            }
    finally:
        connection.close()
    return snapshot


def keychain_metadata_present() -> bool:
    result = subprocess.run(
        [
            "/usr/bin/security",
            "find-generic-password",
            "-s",
            "sk.bagent.app",
            "-a",
            "bagent.tavily.apikey",
        ],
        text=True,
        capture_output=True,
    )
    return result.returncode == 0


def chat(base_url: str, token: str) -> list[dict]:
    request = urllib.request.Request(
        f"{base_url}/chat",
        data=json.dumps({"message": "summarize my latest 3 emails"}).encode(),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    events: list[dict] = []
    with urllib.request.urlopen(request, timeout=360) as response:
        for raw_line in response:
            line = raw_line.decode(errors="replace").strip()
            if not line.startswith("data:"):
                continue
            try:
                event = json.loads(line.removeprefix("data:").strip())
            except json.JSONDecodeError:
                continue
            if isinstance(event, dict):
                events.append(event)
    return events


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--app-bundle", required=True, type=pathlib.Path)
    parser.add_argument(
        "--data-dir",
        type=pathlib.Path,
        default=pathlib.Path.home() / "Library/Application Support/bagent",
    )
    parser.add_argument(
        "--launch-agent",
        type=pathlib.Path,
        default=pathlib.Path.home() / "Library/LaunchAgents/com.bagent.daemon.plist",
    )
    parser.add_argument("--output", required=True, type=pathlib.Path)
    args = parser.parse_args()

    if not args.app_bundle.exists():
        raise RuntimeError("signed app bundle is missing")

    keychain_before = keychain_metadata_present()
    run("/bin/launchctl", "setenv", "BAGENT_EVIDENCE_ORCHESTRATOR", "0")
    try:
        subprocess.run(
            ["/usr/bin/osascript", "-e", 'tell application id "sk.bagent.app" to quit'],
            text=True,
            capture_output=True,
        )
        run("/usr/bin/open", "-n", str(args.app_bundle))
        port, token = wait_for_daemon(args.data_dir)
        time.sleep(12)

        with args.launch_agent.open("rb") as plist_file:
            environment = plistlib.load(plist_file).get("EnvironmentVariables", {})
        if environment.get("BAGENT_EVIDENCE_ORCHESTRATOR") != "0":
            raise AssertionError("signed daemon did not inherit the rollback value")
        if "BAGENT_STAGE8_ACCEPTANCE_FIXTURES" in environment:
            raise AssertionError("ordinary rollback bundle exposed the acceptance fixture flag")

        state_before = protected_snapshot(args.data_dir / "bagent.db")
        events = chat(f"http://127.0.0.1:{port}", token)
        state_after = protected_snapshot(args.data_dir / "bagent.db")

        event_types = [event.get("type") for event in events]
        typed_events = [
            value
            for value in event_types
            if isinstance(value, str)
            and (value.startswith("evidence_") or value.startswith("logical_activity_"))
        ]
        tavily_events = [
            event
            for event in events
            if str(event.get("provider", "")).lower() == "tavily"
        ]
        if typed_events:
            raise AssertionError(f"rollback emitted typed evidence events: {typed_events}")
        if tavily_events:
            raise AssertionError("rollback emitted Tavily provider activity")
        if event_types.count("done") != 1:
            raise AssertionError("rollback did not emit exactly one done event")
        if "token" not in event_types:
            raise AssertionError("prior loop did not return a response token")
        if state_before != state_after:
            changed = sorted(
                table
                for table in PROTECTED_TABLES
                if state_before.get(table) != state_after.get(table)
            )
            raise AssertionError(f"rollback request changed protected state: {changed}")
        if keychain_metadata_present() != keychain_before:
            raise AssertionError("rollback changed Tavily Keychain item presence")

        args.output.write_text(
            json.dumps(
                {
                    "done_count": event_types.count("done"),
                    "legacy_tool_call_count": event_types.count("tool_call"),
                    "protected_state": state_after,
                    "protected_state_unchanged": True,
                    "response_present": True,
                    "tavily_activity_count": 0,
                    "tavily_keychain_item_preserved": True,
                    "typed_evidence_event_count": 0,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n"
        )
    finally:
        subprocess.run(
            ["/bin/launchctl", "unsetenv", "BAGENT_EVIDENCE_ORCHESTRATOR"],
            text=True,
            capture_output=True,
        )
        subprocess.run(
            ["/usr/bin/osascript", "-e", 'tell application id "sk.bagent.app" to quit'],
            text=True,
            capture_output=True,
        )
        subprocess.run(
            ["/usr/bin/open", "-n", str(args.app_bundle)],
            text=True,
            capture_output=True,
        )


if __name__ == "__main__":
    main()

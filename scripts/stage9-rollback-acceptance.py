#!/usr/bin/env python3
"""Verify signed Stage 9 rollback routing without exposing private payloads."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import plistlib
import sqlite3
import subprocess
import time
import urllib.error
import urllib.request


OPERATIONAL_TABLES = {"audit_entries"}


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


def file_digest(path: pathlib.Path) -> dict[str, object]:
    if not path.exists():
        return {"present": False}
    content = path.read_bytes()
    return {
        "present": True,
        "bytes": len(content),
        "sha256": hashlib.sha256(content).hexdigest(),
    }


def protected_snapshot(data_dir: pathlib.Path) -> dict[str, object]:
    database = data_dir / "bagent.db"
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    tables: dict[str, dict[str, object]] = {}
    try:
        existing = sorted(
            row[0]
            for row in connection.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
            )
        )
        for table in existing:
            if table in OPERATIONAL_TABLES:
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
            tables[table] = {
                "present": True,
                "rows": len(rows),
                "sha256": hashlib.sha256(canonical).hexdigest(),
            }
    finally:
        connection.close()
    attachments = {
        str(path.relative_to(data_dir)): file_digest(path)
        for path in sorted((data_dir / "attachments").glob("**/*"))
        if path.is_file()
    }
    return {
        "tables": tables,
        "rules": file_digest(data_dir / "rules.yaml"),
        "daemon_token": file_digest(data_dir / "daemon.token"),
        "attachments": attachments,
    }


def keychain_metadata() -> dict[str, object]:
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
    metadata = (result.stdout + result.stderr).encode()
    return {
        "present": result.returncode == 0,
        "metadata_sha256": hashlib.sha256(metadata).hexdigest(),
    }


def existing_session_id(data_dir: pathlib.Path) -> str:
    connection = sqlite3.connect(f"file:{data_dir / 'bagent.db'}?mode=ro", uri=True)
    try:
        row = connection.execute(
            "SELECT id FROM sessions ORDER BY started_at LIMIT 1"
        ).fetchone()
    finally:
        connection.close()
    if row is None:
        raise RuntimeError("rollback acceptance requires one existing opaque session")
    return str(row[0])


def chat(base_url: str, token: str, session_id: str) -> dict[str, object]:
    request = urllib.request.Request(
        f"{base_url}/chat",
        data=json.dumps(
            {
                "message": "summarize my latest 3 emails",
                "session_id": session_id,
            }
        ).encode(),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    event_types: list[str] = []
    error_codes: list[str] = []
    tavily_activity_count = 0
    response_present = False
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
                event_type = event.get("type")
                if isinstance(event_type, str):
                    event_types.append(event_type)
                    response_present = response_present or event_type == "token"
                    if event_type == "error" and isinstance(event.get("code"), str):
                        error_codes.append(event["code"])
                if str(event.get("provider", "")).lower() == "tavily":
                    tavily_activity_count += 1
                del event
    return {
        "event_types": event_types,
        "error_codes": error_codes,
        "response_present": response_present,
        "tavily_activity_count": tavily_activity_count,
    }


def launch_agent_environment(path: pathlib.Path) -> dict[str, str]:
    with path.open("rb") as plist_file:
        return plistlib.load(plist_file).get("EnvironmentVariables", {})


def restart_and_wait(
    app_bundle: pathlib.Path,
    data_dir: pathlib.Path,
    launch_agent: pathlib.Path,
    previous_pid: str,
    expected_evidence_value: str | None,
) -> tuple[str, str, str]:
    run("/usr/bin/osascript", "-e", 'tell application id "sk.bagent.app" to quit')
    run("/usr/bin/open", "-n", str(app_bundle))
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        try:
            pid = scalar(data_dir / "daemon.pid")
            environment = launch_agent_environment(launch_agent)
            actual = environment.get("BAGENT_EVIDENCE_ORCHESTRATOR")
            if pid != previous_pid and actual == expected_evidence_value:
                port, token = wait_for_daemon(data_dir, timeout=3)
                return port, token, pid
        except (OSError, RuntimeError, plistlib.InvalidFileException):
            pass
        time.sleep(0.25)
    raise RuntimeError("signed daemon did not restart with the expected routing configuration")


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

    initial_environment = launch_agent_environment(args.launch_agent)
    if "BAGENT_STAGE8_ACCEPTANCE_FIXTURES" in initial_environment:
        raise AssertionError("ordinary bundle exposed the acceptance fixture flag")
    initial_pid = scalar(args.data_dir / "daemon.pid")
    state_before_activation = protected_snapshot(args.data_dir)
    keychain_before = keychain_metadata()
    if not keychain_before["present"]:
        raise AssertionError("Tavily Keychain item was absent before rollback acceptance")
    run("/bin/launchctl", "setenv", "BAGENT_EVIDENCE_ORCHESTRATOR", "0")
    try:
        port, token, rollback_pid = restart_and_wait(
            args.app_bundle,
            args.data_dir,
            args.launch_agent,
            initial_pid,
            "0",
        )
        time.sleep(12)

        environment = launch_agent_environment(args.launch_agent)
        if environment.get("BAGENT_EVIDENCE_ORCHESTRATOR") != "0":
            raise AssertionError("signed daemon did not inherit the rollback value")
        if "BAGENT_STAGE8_ACCEPTANCE_FIXTURES" in environment:
            raise AssertionError("ordinary rollback bundle exposed the acceptance fixture flag")

        state_after_activation = protected_snapshot(args.data_dir)
        if state_before_activation != state_after_activation:
            raise AssertionError("rollback activation changed protected state or schema")
        if keychain_metadata() != keychain_before:
            raise AssertionError("rollback activation changed Tavily Keychain metadata")

        event_summary = chat(
            f"http://127.0.0.1:{port}", token, existing_session_id(args.data_dir)
        )
        state_after_turn = protected_snapshot(args.data_dir)

        event_types = event_summary["event_types"]
        typed_events = [
            value
            for value in event_types
            if isinstance(value, str)
            and (value.startswith("evidence_") or value.startswith("logical_activity_"))
        ]
        if typed_events:
            raise AssertionError(f"rollback emitted typed evidence events: {typed_events}")
        if event_summary["tavily_activity_count"]:
            raise AssertionError("rollback emitted Tavily provider activity")
        terminal_count = event_types.count("done") + event_types.count("error")
        if terminal_count != 1:
            raise AssertionError("rollback did not emit exactly one safe legacy terminal")
        if not event_summary["response_present"] and "error" not in event_types:
            raise AssertionError("prior loop produced neither a response nor safe error")
        safe_error_codes = {
            "model_unavailable_metal_oom",
            "model_unavailable_metal_device",
            "model_unavailable_metal_command_buffer",
            "model_unavailable_timeout",
        }
        error_codes = event_summary["error_codes"]
        if "error" in event_types and (
            len(error_codes) != 1 or error_codes[0] not in safe_error_codes
        ):
            raise AssertionError("rollback emitted an unknown or unsafe legacy error code")
        if "tool_call" not in event_types and not event_summary["response_present"]:
            raise AssertionError("rollback did not demonstrate the prior agentic loop")
        if state_before_activation != state_after_turn:
            raise AssertionError("rollback turn changed protected stored user state")
        if keychain_metadata() != keychain_before:
            raise AssertionError("rollback turn changed Tavily Keychain metadata")

        args.output.write_text(
            json.dumps(
                {
                    "done_count": event_types.count("done"),
                    "error_count": event_types.count("error"),
                    "error_code": error_codes[0] if error_codes else None,
                    "legacy_tool_call_count": event_types.count("tool_call"),
                    "protected_state": state_after_turn,
                    "rollback_activation_state_unchanged": True,
                    "protected_state_unchanged": True,
                    "response_present": event_summary["response_present"],
                    "safe_legacy_terminal_count": terminal_count,
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
        run("/bin/launchctl", "unsetenv", "BAGENT_EVIDENCE_ORCHESTRATOR")
        _, _, _ = restart_and_wait(
            args.app_bundle,
            args.data_dir,
            args.launch_agent,
            locals().get("rollback_pid", initial_pid),
            None,
        )
        restored_environment = launch_agent_environment(args.launch_agent)
        if "BAGENT_EVIDENCE_ORCHESTRATOR" in restored_environment:
            raise AssertionError("rollback restoration left the routing flag in the LaunchAgent")
        if "BAGENT_STAGE8_ACCEPTANCE_FIXTURES" in restored_environment:
            raise AssertionError("rollback restoration exposed the acceptance fixture flag")


if __name__ == "__main__":
    main()

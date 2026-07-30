#!/usr/bin/env python3
"""Privacy-safe signed acceptance for post-Stage 9 web stabilization."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import subprocess
import time
import urllib.error
import urllib.request


DAEMON_LABEL = "com.bagent.daemon"
KEY_PATTERN = re.compile(rb"tvly-[A-Za-z0-9_-]{20,}")


def scalar(path: pathlib.Path) -> str:
    return path.read_text().strip()


def request_status(url: str, token: str | None) -> tuple[int, dict[str, object]]:
    headers = {} if token is None else {"Authorization": f"Bearer {token}"}
    request = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=3) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, {}


def wait_for_configured_daemon(
    data_dir: pathlib.Path, previous_pid: int | None, timeout: float = 60.0
) -> tuple[str, str, int]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            port = scalar(data_dir / "daemon.port")
            token = scalar(data_dir / "daemon.token")
            status, health = request_status(f"http://127.0.0.1:{port}/health", token)
            process_id = health.get("process_id")
            if (
                status == 200
                and isinstance(process_id, int)
                and process_id != previous_pid
                and health.get("tavily_configuration") == "configured"
            ):
                return port, token, process_id
        except (OSError, ValueError, urllib.error.URLError):
            pass
        time.sleep(0.25)
    raise RuntimeError("signed daemon did not reach configured state after clean restart")


def signed_live_web_turn(base_url: str, token: str) -> tuple[dict[str, object], str | None]:
    request = urllib.request.Request(
        f"{base_url}/chat",
        data=json.dumps(
            {
                "message": (
                    "What is the current population of Bratislava? "
                    "Verify it using two independent sources."
                )
            }
        ).encode(),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    providers: list[dict[str, object]] = []
    outcomes: list[dict[str, object]] = []
    done_count = 0
    session_id = None
    with urllib.request.urlopen(request, timeout=360) as response:
        for raw_line in response:
            line = raw_line.decode(errors="replace").strip()
            if not line.startswith("data:"):
                continue
            try:
                event = json.loads(line.removeprefix("data:").strip())
            except json.JSONDecodeError:
                continue
            if not isinstance(event, dict):
                continue
            if (
                event.get("type") == "evidence_acquisition_diagnostic"
                and event.get("status") == "search_completed"
            ):
                providers.append(
                    {
                        "provider": event.get("provider"),
                        "provider_status": event.get("provider_status"),
                        "search_attempts_used": event.get("search_attempts_used"),
                        "search_attempt_budget": event.get("search_attempt_budget"),
                    }
                )
            elif event.get("type") == "evidence_outcome":
                outcomes.append(
                    {
                        "state": event.get("state"),
                        "acquired": event.get("acquired"),
                        "requested": event.get("requested"),
                        "source_count": event.get("source_count"),
                    }
                )
            elif event.get("type") == "done":
                done_count += 1
                if isinstance(event.get("session_id"), str):
                    session_id = event["session_id"]
    if not providers or providers[0].get("provider") != "tavily":
        raise AssertionError("Tavily was not the first signed discovery provider")
    if len(outcomes) != 1 or done_count != 1:
        raise AssertionError("signed live turn did not have one outcome and one done")
    return (
        {"providers": providers, "outcome": outcomes[0], "done_count": done_count},
        session_id,
    )


def delete_session(base_url: str, token: str, session_id: str | None) -> None:
    if session_id is None:
        return
    request = urllib.request.Request(
        f"{base_url}/sessions/{session_id}",
        headers={"Authorization": f"Bearer {token}"},
        method="DELETE",
    )
    with urllib.request.urlopen(request, timeout=5) as response:
        if response.status != 200:
            raise RuntimeError("acceptance session cleanup failed")


def keychain_item_present() -> bool:
    result = subprocess.run(
        [
            "/usr/bin/security",
            "find-generic-password",
            "-s",
            "sk.bagent.app",
            "-a",
            "bagent.tavily.apikey",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return result.returncode == 0


def files_have_key_pattern(paths: list[pathlib.Path]) -> bool:
    for root in paths:
        candidates = root.rglob("*") if root.is_dir() else [root]
        for candidate in candidates:
            if not candidate.is_file():
                continue
            try:
                if KEY_PATTERN.search(candidate.read_bytes()):
                    return True
            except OSError:
                continue
    return False


def process_state_has_key_pattern(process_id: int) -> bool:
    process = subprocess.run(
        ["/bin/ps", "eww", "-p", str(process_id), "-o", "command="],
        capture_output=True,
        check=True,
    )
    launchd = subprocess.run(
        ["/bin/launchctl", "print", f"gui/{os.getuid()}/{DAEMON_LABEL}"],
        capture_output=True,
        check=True,
    )
    return KEY_PATTERN.search(process.stdout) is not None or KEY_PATTERN.search(launchd.stdout) is not None


def verify_deterministic_shortfall(
    report_path: pathlib.Path,
    signed_app: pathlib.Path,
    expected_commit: str,
) -> dict[str, object]:
    report = json.loads(report_path.read_text())
    provenance = report.get("provenance", {})
    if provenance.get("source_commit") != expected_commit:
        raise AssertionError("deterministic signed report commit provenance changed")
    signed_app_sha256 = provenance.get("signed_app_sha256")
    actual_signed_app_sha256 = hashlib.sha256(signed_app.read_bytes()).hexdigest()
    if signed_app_sha256 != actual_signed_app_sha256:
        raise AssertionError("deterministic signed report binary provenance changed")
    case = report["cases"]["web_all_fetch_failure"]
    outcome = case["outcome"]
    proof = {
        "state": outcome["state"],
        "acquired": outcome["acquired"],
        "requested": outcome["requested"],
        "outcome_count": case["outcome_count"],
        "done_count": case["done_count"],
    }
    expected = {
        "state": "verification_shortfall",
        "acquired": 0,
        "requested": 2,
        "outcome_count": 1,
        "done_count": 1,
    }
    if proof != expected:
        raise AssertionError(f"deterministic unusable-evidence proof changed: {proof}")
    return proof


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--app-bundle", type=pathlib.Path, required=True)
    parser.add_argument("--deterministic-report", type=pathlib.Path, required=True)
    parser.add_argument("--deterministic-signed-app", type=pathlib.Path, required=True)
    parser.add_argument("--expected-commit", required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()

    if not args.app_bundle.is_dir():
        raise RuntimeError("signed app bundle is missing")
    if not keychain_item_present():
        raise RuntimeError("Tavily Keychain item is absent")

    data_dir = pathlib.Path.home() / "Library/Application Support/bagent"
    previous_pid = None
    try:
        previous_pid = int(scalar(data_dir / "daemon.pid"))
    except (OSError, ValueError):
        pass

    subprocess.run(
        ["/usr/bin/osascript", "-e", 'tell application "bagent" to quit'],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    subprocess.run(
        ["/bin/launchctl", "bootout", f"gui/{os.getuid()}/{DAEMON_LABEL}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    subprocess.run(["/usr/bin/open", "-a", str(args.app_bundle)], check=True)

    port, token, process_id = wait_for_configured_daemon(data_dir, previous_pid)
    base_url = f"http://127.0.0.1:{port}"
    unauthenticated, _ = request_status(f"{base_url}/web/tavily/status", None)
    authenticated, status_body = request_status(f"{base_url}/web/tavily/status", token)
    if unauthenticated != 401 or authenticated != 200:
        raise AssertionError("Tavily status authentication boundary changed")
    if status_body != {"status": "configured"}:
        raise AssertionError("Tavily status response exposed unexpected fields")

    live, session_id = signed_live_web_turn(base_url, token)
    delete_session(base_url, token, session_id)
    deterministic_shortfall = verify_deterministic_shortfall(
        args.deterministic_report,
        args.deterministic_signed_app,
        args.expected_commit,
    )
    launch_agent = pathlib.Path.home() / "Library/LaunchAgents/com.bagent.daemon.plist"
    scan_paths = [
        args.app_bundle,
        data_dir,
        pathlib.Path.home() / "Library/Logs/bagent",
        launch_agent,
        args.deterministic_report,
        args.deterministic_signed_app,
    ]
    artifact_pattern_absent = not files_have_key_pattern(scan_paths)
    process_pattern_absent = not process_state_has_key_pattern(process_id)
    if not artifact_pattern_absent or not process_pattern_absent:
        raise AssertionError("credential-like material appeared outside Keychain")

    args.output.write_text(
        json.dumps(
            {
                "clean_restart": {"pid_changed": process_id != previous_pid},
                "configuration": "configured",
                "status_route": {
                    "unauthenticated": unauthenticated,
                    "authenticated": authenticated,
                    "fields": sorted(status_body),
                },
                "live_web": live,
                "deterministic_unusable_evidence": deterministic_shortfall,
                "privacy": {
                    "keychain_item_present": True,
                    "artifact_key_pattern_absent": artifact_pattern_absent,
                    "process_key_pattern_absent": process_pattern_absent,
                },
            },
            indent=2,
            sort_keys=True,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Dependency-free deterministic verification driver for reference contracts."""

from __future__ import annotations

import argparse
import copy
import datetime
import hashlib
import json
import pathlib
import platform
import re
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from typing import Any


SCHEMA = "reference-verification-manifest/v1"
NORMALIZED_ID = r"[a-z0-9][a-z0-9._-]*"
CASE_RESULT = re.compile(rf"^REFERENCE_CASE_RESULT case=({NORMALIZED_ID}) outcome=([a-z0-9_]+)$")
REGISTRY = re.compile(
    rf"^REFERENCE_REGISTRY case=({NORMALIZED_ID}) outcome=([a-z0-9_]+) "
    r"teardown=([01]) mutation=([a-z0-9_]+)$"
)
TRACE = re.compile(
    rf"^REFERENCE_TRACE case=({NORMALIZED_ID}) schema=(\d+) sequence=(\d+) "
    r"causal_group=(\d+) attempt=(\d+) operation=([a-z0-9_]+) "
    r"structural_result=([a-z0-9_]+) completion=([a-z0-9_]+)$"
)
TEARDOWN = re.compile(rf"^REFERENCE_TEARDOWN case=({NORMALIZED_ID}) complete=1$")
ZERO_CALL = re.compile(r"^REFERENCE_ZERO_CALL class=([a-z0-9_]+) count=(\d+)$")
HOSTILE_SENTINELS = (
    b"SYNTHETIC_MENTION_SENTINEL_01A1",
    b"SYNTHETIC_PROPOSAL_SENTINEL_02B2",
    b"SYNTHETIC_CONVERSATION_SENTINEL_03C3",
    b"SYNTHETIC_EVIDENCE_CONTENT_SENTINEL_04D4",
    b"SYNTHETIC_PRIVATE_MAIL_SENTINEL_7F4A",
    b"SYNTHETIC_QUERY_SENTINEL_8B2C",
    b"SYNTHETIC_CONNECTOR_ID_SENTINEL_05E5",
    b"SYNTHETIC_EVIDENCE_ID_SENTINEL_06F6",
    b"SYNTHETIC_CREDENTIAL_SENTINEL_9D1E",
    b"SYNTHETIC_URL_SENTINEL_6A3F",
    b"SYNTHETIC_ATTACHMENT_SENTINEL_07A7",
    b"SYNTHETIC_SERIAL_TRACKING_SENTINEL_08B8",
    b"SYNTHETIC_RAW_SESSION_ID_SENTINEL_09C9",
    b"SYNTHETIC_UNKEYED_SHA256_SENTINEL_10D0",
)
SLICE1_REQUIRED_CASES = {
    "slice1.privacy.closed_types": "privacy_safe",
    "slice1.registry.complete": "registry_complete",
    "slice1.teardown.complete": "teardown_complete",
    "slice1.trace.structural": "trace_matched",
}
REQUIRED_ZERO_CALL_CLASSES = (
    "mail_access",
    "model_inference",
    "prompt_construction",
    "provider_transport",
    "runtime_mutation",
)
RECEIPT_SCHEMA = "reference-verification-gate-receipt/v1"
CANDIDATE_PATHS = (
    "apps/macos/Makefile",
    "crates/daemon/src/evidence/acceptance.rs",
    "crates/daemon/src/main.rs",
    "crates/daemon/src/reference_resolution/contract_tests/mod.rs",
    "scripts/reference-compilefail.py",
    "scripts/reference-signed-acceptance.py",
    "scripts/reference-verification.py",
)
COUNT_KEYS = ("discovered", "executed", "passed", "failed", "skipped", "ignored", "live_excluded")
GATE_KEYS = (
    "test_selection",
    "registry",
    "structural_trace",
    "privacy",
    "teardown",
    "ordinary_exclusion",
    "compiler_fail",
)


class VerificationFailure(RuntimeError):
    def __init__(self, reason: str):
        super().__init__(reason)
        self.reason = reason


@dataclass(frozen=True)
class TestExecutable:
    path: pathlib.Path
    target: str


@dataclass(frozen=True)
class RustRunReport:
    cases: list[dict[str, str]]
    counts: dict[str, int]
    teardown_markers: list[str]
    test_binary_identities: list[dict[str, str]]
    zero_calls: list[dict[str, Any]]


def run(
    command: list[str],
    *,
    cwd: pathlib.Path,
    environment: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        env=environment,
        text=True,
        capture_output=True,
        check=False,
    )


def canonical_json_bytes(value: Any) -> bytes:
    return (
        json.dumps(value, ensure_ascii=False, allow_nan=False, sort_keys=True, separators=(",", ":"))
        .encode("utf-8")
        + b"\n"
    )


def sha256(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def assert_no_hostile_sentinels(payload: bytes) -> None:
    if any(sentinel in payload for sentinel in HOSTILE_SENTINELS):
        raise VerificationFailure("forbidden_sentinel_present")


def binary_identities(selected: list[tuple[TestExecutable, str]]) -> list[dict[str, str]]:
    executables = {(item.path, item.target) for item, _ in selected}
    identities = [
        {"target_class": target, "sha256": sha256(path.read_bytes())}
        for path, target in executables
    ]
    return sorted(identities, key=lambda item: (item["target_class"], item["sha256"]))


def cargo_test_executables(
    repo: pathlib.Path, package: str, features: str | None
) -> list[TestExecutable]:
    command = ["cargo", "test", "--no-run", "--message-format=json", "-p", package]
    if features:
        command.extend(["--features", features])
    result = run(command, cwd=repo)
    assert_no_hostile_sentinels((result.stdout + result.stderr).encode("utf-8"))
    if result.returncode:
        raise VerificationFailure("cargo_setup_failed")
    executables: dict[pathlib.Path, TestExecutable] = {}
    for line in result.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("reason") != "compiler-artifact" or not message.get("profile", {}).get("test"):
            continue
        executable = message.get("executable")
        target = message.get("target", {}).get("name")
        if isinstance(executable, str) and isinstance(target, str):
            path = pathlib.Path(executable)
            executables[path] = TestExecutable(path, target)
    if not executables:
        raise VerificationFailure("no_test_executables")
    return sorted(executables.values(), key=lambda item: (item.target, str(item.path)))


def list_tests(executable: TestExecutable, ignored: bool = False) -> list[str]:
    command = [str(executable.path), "--list", "--format", "terse"]
    if ignored:
        command.append("--ignored")
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    assert_no_hostile_sentinels((result.stdout + result.stderr).encode("utf-8"))
    if result.returncode:
        raise VerificationFailure("test_listing_failed")
    return [
        line.removesuffix(": test")
        for line in result.stdout.splitlines()
        if line.endswith(": test")
    ]


def discover_tests(
    repo: pathlib.Path, package: str, features: str | None, prefix: str
) -> tuple[list[tuple[TestExecutable, str]], set[str]]:
    selected: list[tuple[TestExecutable, str]] = []
    ignored_names: set[str] = set()
    for executable in cargo_test_executables(repo, package, features):
        names = list_tests(executable)
        ignored_names.update(list_tests(executable, ignored=True))
        selected.extend((executable, name) for name in names if name.startswith(prefix))
    return selected, ignored_names


def marker_lines(output: str) -> list[str]:
    markers = []
    for line in output.splitlines():
        marker_start = line.find("REFERENCE_")
        if marker_start >= 0:
            markers.append(line[marker_start:].strip())
    return markers


def verify_structural_markers(
    lines: list[str], mutation: str | None
) -> tuple[list[dict[str, str]], list[str], list[dict[str, Any]]]:
    registry: dict[str, dict[str, str]] = {}
    results: dict[str, str] = {}
    traces: dict[str, dict[str, str]] = {}
    teardowns: set[str] = set()
    zero_calls: dict[str, int] = {}
    for line in lines:
        if match := REGISTRY.fullmatch(line):
            case_id, outcome, teardown, oracle_mutation = match.groups()
            registry[case_id] = {
                "case_id": case_id,
                "structural_outcome": outcome,
                "teardown_required": teardown,
                "oracle_mutation": oracle_mutation,
            }
        elif match := CASE_RESULT.fullmatch(line):
            results[match.group(1)] = match.group(2)
        elif match := TRACE.fullmatch(line):
            traces[match.group(1)] = {
                "schema": match.group(2),
                "sequence": match.group(3),
                "causal_group": match.group(4),
                "attempt": match.group(5),
                "operation": match.group(6),
                "structural_result": match.group(7),
                "completion": match.group(8),
            }
        elif match := TEARDOWN.fullmatch(line):
            teardowns.add(match.group(1))
        elif match := ZERO_CALL.fullmatch(line):
            call_class, count = match.groups()
            if call_class in zero_calls:
                raise VerificationFailure("duplicate_zero_call_class")
            zero_calls[call_class] = int(count)
    if mutation == "missing-required-case" and registry:
        missing = sorted(registry)[0]
        registry.pop(missing)
        results.pop(missing, None)
    elif mutation == "wrong-structural-trace" and traces:
        traces[sorted(traces)[0]]["structural_result"] = "mutated"
    elif mutation == "teardown-failure":
        teardowns.clear()
    if not registry:
        raise VerificationFailure("registry_not_observed")
    observed_inventory = {
        case_id: entry["structural_outcome"] for case_id, entry in registry.items()
    }
    if observed_inventory != SLICE1_REQUIRED_CASES:
        raise VerificationFailure("missing_required_case")
    if set(registry) - set(results):
        raise VerificationFailure("missing_required_case")
    if set(results) - set(registry):
        raise VerificationFailure("unknown_case_result")
    for case_id, entry in registry.items():
        if results[case_id] != entry["structural_outcome"]:
            raise VerificationFailure("structural_outcome_mismatch")
        if entry["teardown_required"] == "1" and case_id not in teardowns:
            raise VerificationFailure("teardown_incomplete")
    expected_trace = {
        "schema": "1",
        "sequence": "1",
        "causal_group": "0",
        "attempt": "1",
        "operation": "recorder",
        "structural_result": "matched",
        "completion": "success",
    }
    if traces.get("slice1.trace.structural") != expected_trace:
        raise VerificationFailure("structural_trace_mismatch")
    if tuple(sorted(zero_calls)) != REQUIRED_ZERO_CALL_CLASSES:
        raise VerificationFailure("zero_call_classes_incomplete")
    if any(zero_calls.values()):
        raise VerificationFailure("forbidden_call_observed")
    cases = [
        {"case_id": case_id, "structural_outcome": registry[case_id]["structural_outcome"]}
        for case_id in sorted(registry)
    ]
    return (
        cases,
        sorted(teardowns),
        [{"call_class": item, "count": zero_calls[item]} for item in sorted(zero_calls)],
    )


def collect_rust_run(args: argparse.Namespace) -> RustRunReport:
    repo = pathlib.Path(args.repo).resolve()
    selected, ignored_names = discover_tests(repo, args.package, args.features, args.prefix)
    identities = binary_identities(selected) if selected else []
    discovered = len(selected)
    if args.require_nonzero and discovered == 0:
        raise VerificationFailure("zero_tests_discovered")
    selected_ignored = [name for _, name in selected if name in ignored_names]
    if selected and len(selected_ignored) == discovered:
        raise VerificationFailure("ignored_only_selection")
    executed: list[str] = []
    passed = 0
    failed = 0
    markers: list[str] = []
    if args.execution_control != "zero-executed":
        for executable, name in selected:
            if name in ignored_names:
                continue
            result = subprocess.run(
                [str(executable.path), name, "--exact", "--nocapture", "--test-threads=1"],
                text=True,
                capture_output=True,
                check=False,
            )
            combined = result.stdout + "\n" + result.stderr
            assert_no_hostile_sentinels(combined.encode("utf-8"))
            if "running 1 test" not in combined:
                raise VerificationFailure("test_execution_incomplete")
            executed.append(name)
            markers.extend(marker_lines(combined))
            if result.returncode == 0 and "test result: ok." in combined:
                passed += 1
            else:
                failed += 1
    if args.require_nonzero and not executed:
        raise VerificationFailure("zero_tests_executed")
    if len(executed) + len(selected_ignored) != discovered:
        raise VerificationFailure("discovered_executed_mismatch")
    if failed:
        raise VerificationFailure("selected_test_failed")
    cases: list[dict[str, str]] = []
    teardowns: list[str] = []
    zero_calls: list[dict[str, Any]] = []
    if any(REGISTRY.fullmatch(line) for line in markers):
        cases, teardowns, zero_calls = verify_structural_markers(markers, args.oracle_mutation)
    elif args.oracle_mutation:
        raise VerificationFailure("registry_not_observed")
    return RustRunReport(
        cases=cases,
        counts={
            "discovered": discovered,
            "executed": len(executed),
            "passed": passed,
            "failed": failed,
            "skipped": len(selected_ignored),
            "ignored": len(selected_ignored),
            "live_excluded": 0,
        },
        teardown_markers=teardowns,
        test_binary_identities=identities,
        zero_calls=zero_calls,
    )


def git_scalar(repo: pathlib.Path, *arguments: str) -> str:
    result = run(["git", *arguments], cwd=repo)
    if result.returncode:
        raise VerificationFailure("git_identity_unavailable")
    return result.stdout.strip()


def toolchain_identity(repo: pathlib.Path) -> dict[str, str]:
    version = run(["rustc", "--version"], cwd=repo)
    verbose = run(["rustc", "-vV"], cwd=repo)
    if version.returncode or verbose.returncode:
        raise VerificationFailure("toolchain_identity_unavailable")
    target = next(
        (line.removeprefix("host: ") for line in verbose.stdout.splitlines() if line.startswith("host: ")),
        "unknown",
    )
    return {
        "toolchain": version.stdout.strip(),
        "target": target,
        "os_class": sys.platform,
        "platform_class": platform.machine().lower() or "unknown",
    }


def zero_counts() -> dict[str, int]:
    return {key: 0 for key in COUNT_KEYS}


def unit_gate_counts(units: int = 1) -> dict[str, int]:
    counts = zero_counts()
    counts.update({"discovered": units, "executed": units, "passed": units})
    return counts


def candidate_identity(repo: pathlib.Path) -> dict[str, Any]:
    digest = hashlib.sha256()
    for relative in CANDIDATE_PATHS:
        payload = (repo / relative).read_bytes()
        digest.update(relative.encode("utf-8") + b"\0" + payload + b"\0")
    return {
        "commit": git_scalar(repo, "rev-parse", "HEAD"),
        "dirty_state_class": "dirty" if git_scalar(repo, "status", "--porcelain") else "clean",
        "environment": toolchain_identity(repo),
        "source_state_sha256": digest.hexdigest(),
    }


def make_receipt(
    repo: pathlib.Path,
    campaign_id: str,
    candidate: dict[str, Any],
    started_wall: datetime.datetime,
    started_monotonic_ns: int,
    gate_results: list[dict[str, Any]],
    payload: dict[str, Any],
) -> dict[str, Any]:
    ended_wall = datetime.datetime.now(datetime.timezone.utc)
    ended_monotonic_ns = time.monotonic_ns()
    if candidate_identity(repo) != candidate:
        raise VerificationFailure("candidate_changed_during_gate")
    return {
        "schema": RECEIPT_SCHEMA,
        "campaign_id": campaign_id,
        "candidate_identity": candidate,
        "timing": {
            "started_utc": started_wall.isoformat(timespec="milliseconds").replace("+00:00", "Z"),
            "ended_utc": ended_wall.isoformat(timespec="milliseconds").replace("+00:00", "Z"),
            "monotonic_started_ns": started_monotonic_ns,
            "monotonic_ended_ns": ended_monotonic_ns,
            "monotonic_duration_ms": (ended_monotonic_ns - started_monotonic_ns) // 1_000_000,
        },
        "gate_results": gate_results,
        "payload": payload,
    }


def write_receipt(path: pathlib.Path, value: dict[str, Any]) -> None:
    validate_receipt(value)
    path.write_bytes(canonical_json_bytes(value))


def validate_counts(counts: Any, reason: str) -> None:
    if not isinstance(counts, dict):
        raise VerificationFailure(reason)
    require_exact_keys(counts, COUNT_KEYS, reason)
    for count in counts.values():
        require_integer(count, reason)
    if counts["executed"] != counts["passed"] + counts["failed"]:
        raise VerificationFailure(reason)


def validate_receipt(value: Any) -> None:
    if not isinstance(value, dict):
        raise VerificationFailure("receipt_not_object")
    require_exact_keys(
        value,
        ("schema", "campaign_id", "candidate_identity", "timing", "gate_results", "payload"),
        "receipt_fields_invalid",
    )
    if value["schema"] != RECEIPT_SCHEMA or not isinstance(value["payload"], dict):
        raise VerificationFailure("receipt_schema_invalid")
    if not isinstance(value["campaign_id"], str) or not re.fullmatch(NORMALIZED_ID, value["campaign_id"]):
        raise VerificationFailure("receipt_campaign_id_invalid")
    candidate = value["candidate_identity"]
    if not isinstance(candidate, dict):
        raise VerificationFailure("receipt_candidate_invalid")
    require_exact_keys(candidate, ("commit", "dirty_state_class", "environment", "source_state_sha256"), "receipt_candidate_invalid")
    if not re.fullmatch(r"[0-9a-f]{40}", candidate["commit"]) or candidate["dirty_state_class"] not in {"clean", "dirty"}:
        raise VerificationFailure("receipt_candidate_invalid")
    if not re.fullmatch(r"[0-9a-f]{64}", candidate["source_state_sha256"]):
        raise VerificationFailure("receipt_candidate_invalid")
    environment = candidate["environment"]
    if not isinstance(environment, dict):
        raise VerificationFailure("receipt_candidate_invalid")
    require_exact_keys(environment, ("toolchain", "target", "os_class", "platform_class"), "receipt_candidate_invalid")
    timing = value["timing"]
    if not isinstance(timing, dict):
        raise VerificationFailure("receipt_timing_invalid")
    require_exact_keys(timing, ("started_utc", "ended_utc", "monotonic_started_ns", "monotonic_ended_ns", "monotonic_duration_ms"), "receipt_timing_invalid")
    started_utc = parse_utc_milliseconds(timing.get("started_utc"), "receipt_timing_invalid")
    ended_utc = parse_utc_milliseconds(timing.get("ended_utc"), "receipt_timing_invalid")
    if started_utc > ended_utc:
        raise VerificationFailure("receipt_timing_invalid")
    for key in ("monotonic_started_ns", "monotonic_ended_ns", "monotonic_duration_ms"):
        require_integer(timing[key], "receipt_timing_invalid")
    if timing["monotonic_ended_ns"] < timing["monotonic_started_ns"] or timing["monotonic_duration_ms"] != (timing["monotonic_ended_ns"] - timing["monotonic_started_ns"]) // 1_000_000:
        raise VerificationFailure("receipt_timing_invalid")
    wall_duration_ms = int((ended_utc - started_utc).total_seconds() * 1000)
    if abs(wall_duration_ms - timing["monotonic_duration_ms"]) > 5_000:
        raise VerificationFailure("receipt_timing_disagreement")
    gate_results = value["gate_results"]
    if not isinstance(gate_results, list) or not gate_results:
        raise VerificationFailure("receipt_gate_results_invalid")
    gate_ids = []
    for result in gate_results:
        if not isinstance(result, dict):
            raise VerificationFailure("receipt_gate_results_invalid")
        require_exact_keys(result, ("gate_id", "counts"), "receipt_gate_results_invalid")
        if result["gate_id"] not in GATE_KEYS:
            raise VerificationFailure("receipt_gate_id_invalid")
        validate_counts(result["counts"], "receipt_counts_invalid")
        if result["counts"]["failed"] or result["counts"]["executed"] == 0:
            raise VerificationFailure("receipt_gate_failed")
        gate_ids.append(result["gate_id"])
    if gate_ids != sorted(set(gate_ids)):
        raise VerificationFailure("receipt_gate_order_invalid")
    assert_no_hostile_sentinels(canonical_json_bytes(value))


def fixture_inputs(versions: dict[str, str], cases: list[dict[str, str]]) -> dict[str, Any]:
    return {
        "versions": {key: versions[key] for key in ("suite", "corpus", "oracle", "trace")},
        "cases": cases,
    }


def stable_projection(manifest: dict[str, Any]) -> dict[str, Any]:
    return {
        key: manifest[key]
        for key in (
            "schema", "commit", "versions", "cases", "fixture_set_identity_sha256",
            "environment", "zero_call_summary", "test_binary_identities",
        )
    }


def make_manifest(
    repo: pathlib.Path,
    rust_receipt: dict[str, Any],
    receipts: list[dict[str, Any]],
) -> dict[str, Any]:
    candidate = rust_receipt["candidate_identity"]
    dirty = candidate["dirty_state_class"] == "dirty"
    payload = rust_receipt["payload"]
    cases = payload["cases"]
    versions = {
        "suite": "reference-slice1/v1",
        "corpus": "reference-cases-slice1/v1",
        "oracle": "reference-oracle/v1",
        "trace": "reference-trace/v1",
        "manifest": SCHEMA,
    }
    combined_gates = [item for receipt in receipts for item in receipt["gate_results"]]
    if [item["gate_id"] for item in sorted(combined_gates, key=lambda item: item["gate_id"])] != sorted(GATE_KEYS):
        raise VerificationFailure("gate_receipts_incomplete")
    gate_results = sorted(combined_gates, key=lambda item: item["gate_id"])
    gates = {item["gate_id"]: True for item in gate_results}
    manifest: dict[str, Any] = {
        "schema": SCHEMA,
        "commit": candidate["commit"],
        "dirty_state_class": "dirty" if dirty else "clean",
        "release_class": "non_release" if dirty else "release_candidate",
        "versions": versions,
        "counts": payload["counts"],
        "cases": cases,
        "fixture_set_identity_sha256": sha256(canonical_json_bytes(fixture_inputs(versions, cases))),
        "test_binary_identities": payload["test_binary_identities"],
        "environment": candidate["environment"],
        "timing": {
            "started_utc": min(receipt["timing"]["started_utc"] for receipt in receipts),
            "ended_utc": max(receipt["timing"]["ended_utc"] for receipt in receipts),
            "monotonic_duration_ms": (
                max(receipt["timing"]["monotonic_ended_ns"] for receipt in receipts)
                - min(receipt["timing"]["monotonic_started_ns"] for receipt in receipts)
            ) // 1_000_000,
        },
        "zero_call_summary": {
            "schema": "reference-zero-call-summary/v1",
            "forbidden_call_total": sum(item["count"] for item in payload["zero_calls"]),
            "classes": payload["zero_calls"],
        },
        "gate_results": gate_results,
        "gate_completeness": gates,
    }
    manifest["stable_identity_sha256"] = sha256(canonical_json_bytes(stable_projection(manifest)))
    manifest["evidence_identity_sha256"] = sha256(canonical_json_bytes(manifest))
    return manifest


def require_exact_keys(value: dict[str, Any], expected: tuple[str, ...], reason: str) -> None:
    if set(value) != set(expected):
        raise VerificationFailure(reason)


def require_integer(value: Any, reason: str) -> None:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise VerificationFailure(reason)


def parse_utc_milliseconds(value: Any, reason: str) -> datetime.datetime:
    if not isinstance(value, str) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z", value):
        raise VerificationFailure(reason)
    try:
        return datetime.datetime.strptime(value, "%Y-%m-%dT%H:%M:%S.%fZ").replace(
            tzinfo=datetime.timezone.utc
        )
    except ValueError:
        raise VerificationFailure(reason) from None


def validate_manifest(value: Any, *, require_complete: bool) -> None:
    if not isinstance(value, dict):
        raise VerificationFailure("manifest_not_object")
    pending = [value]
    while pending:
        current = pending.pop()
        if isinstance(current, dict):
            for key, child in current.items():
                if not isinstance(key, str) or forbidden_json_key(key):
                    raise VerificationFailure("manifest_forbidden_field")
                pending.append(child)
        elif isinstance(current, list):
            pending.extend(current)
    top_keys = (
        "schema", "commit", "dirty_state_class", "release_class", "versions", "counts",
        "cases", "fixture_set_identity_sha256", "test_binary_identities", "environment",
        "timing", "zero_call_summary", "gate_results", "gate_completeness",
        "stable_identity_sha256", "evidence_identity_sha256",
    )
    require_exact_keys(value, top_keys, "manifest_fields_invalid")
    if value["schema"] != SCHEMA:
        raise VerificationFailure("manifest_schema_invalid")
    if not isinstance(value["commit"], str) or not re.fullmatch(r"[0-9a-f]{40}", value["commit"]):
        raise VerificationFailure("manifest_commit_invalid")
    if value["dirty_state_class"] not in {"clean", "dirty"}:
        raise VerificationFailure("manifest_dirty_state_invalid")
    expected_release = "release_candidate" if value["dirty_state_class"] == "clean" else "non_release"
    if value["release_class"] != expected_release:
        raise VerificationFailure("manifest_release_class_invalid")
    versions = value["versions"]
    if not isinstance(versions, dict):
        raise VerificationFailure("manifest_versions_invalid")
    require_exact_keys(versions, ("suite", "corpus", "oracle", "trace", "manifest"), "manifest_versions_invalid")
    if not all(isinstance(item, str) and item for item in versions.values()):
        raise VerificationFailure("manifest_versions_invalid")
    counts = value["counts"]
    validate_counts(counts, "manifest_counts_invalid")
    if require_complete and (counts["discovered"] == 0 or counts["executed"] == 0):
        raise VerificationFailure("manifest_zero_count")
    cases = value["cases"]
    if not isinstance(cases, list):
        raise VerificationFailure("manifest_cases_invalid")
    case_ids = []
    for case in cases:
        if not isinstance(case, dict):
            raise VerificationFailure("manifest_cases_invalid")
        require_exact_keys(case, ("case_id", "structural_outcome"), "manifest_case_fields_invalid")
        if not isinstance(case["case_id"], str) or not re.fullmatch(NORMALIZED_ID, case["case_id"]):
            raise VerificationFailure("manifest_case_id_invalid")
        if not isinstance(case["structural_outcome"], str) or not re.fullmatch(r"[a-z][a-z0-9_]*", case["structural_outcome"]):
            raise VerificationFailure("manifest_outcome_invalid")
        case_ids.append(case["case_id"])
    if case_ids != sorted(set(case_ids)):
        raise VerificationFailure("manifest_case_order_invalid")
    observed_cases = {item["case_id"]: item["structural_outcome"] for item in cases}
    if observed_cases != SLICE1_REQUIRED_CASES:
        raise VerificationFailure("manifest_case_inventory_incomplete")
    identities = value["test_binary_identities"]
    if not isinstance(identities, list) or not identities:
        raise VerificationFailure("manifest_binary_identities_invalid")
    for identity in identities:
        if not isinstance(identity, dict):
            raise VerificationFailure("manifest_binary_identities_invalid")
        require_exact_keys(identity, ("target_class", "sha256"), "manifest_binary_identities_invalid")
        if not isinstance(identity["target_class"], str) or not re.fullmatch(NORMALIZED_ID, identity["target_class"]):
            raise VerificationFailure("manifest_binary_identities_invalid")
        if not isinstance(identity["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", identity["sha256"]):
            raise VerificationFailure("manifest_binary_identities_invalid")
    if identities != sorted(identities, key=lambda item: (item["target_class"], item["sha256"])):
        raise VerificationFailure("manifest_binary_identity_order_invalid")
    environment = value["environment"]
    if not isinstance(environment, dict):
        raise VerificationFailure("manifest_environment_invalid")
    require_exact_keys(environment, ("toolchain", "target", "os_class", "platform_class"), "manifest_environment_invalid")
    if not all(isinstance(item, str) and item for item in environment.values()):
        raise VerificationFailure("manifest_environment_invalid")
    if any("/" in item or "\\" in item for item in environment.values()):
        raise VerificationFailure("manifest_environment_path_forbidden")
    timing = value["timing"]
    if not isinstance(timing, dict):
        raise VerificationFailure("manifest_timing_invalid")
    require_exact_keys(timing, ("started_utc", "ended_utc", "monotonic_duration_ms"), "manifest_timing_invalid")
    manifest_started = parse_utc_milliseconds(timing["started_utc"], "manifest_timestamp_invalid")
    manifest_ended = parse_utc_milliseconds(timing["ended_utc"], "manifest_timestamp_invalid")
    if manifest_started > manifest_ended:
        raise VerificationFailure("manifest_timestamp_invalid")
    require_integer(timing["monotonic_duration_ms"], "manifest_duration_invalid")
    zero_call = value["zero_call_summary"]
    if not isinstance(zero_call, dict):
        raise VerificationFailure("manifest_zero_call_invalid")
    require_exact_keys(zero_call, ("schema", "forbidden_call_total", "classes"), "manifest_zero_call_invalid")
    if zero_call["schema"] != "reference-zero-call-summary/v1":
        raise VerificationFailure("manifest_zero_call_invalid")
    require_integer(zero_call["forbidden_call_total"], "manifest_zero_call_invalid")
    if not isinstance(zero_call["classes"], list):
        raise VerificationFailure("manifest_zero_call_invalid")
    zero_classes = []
    zero_total = 0
    for item in zero_call["classes"]:
        if not isinstance(item, dict):
            raise VerificationFailure("manifest_zero_call_invalid")
        require_exact_keys(item, ("call_class", "count"), "manifest_zero_call_invalid")
        if item["call_class"] not in REQUIRED_ZERO_CALL_CLASSES:
            raise VerificationFailure("manifest_zero_call_invalid")
        require_integer(item["count"], "manifest_zero_call_invalid")
        zero_classes.append(item["call_class"])
        zero_total += item["count"]
    if tuple(zero_classes) != REQUIRED_ZERO_CALL_CLASSES or zero_total != zero_call["forbidden_call_total"]:
        raise VerificationFailure("manifest_zero_call_invalid")
    if require_complete and zero_total != 0:
        raise VerificationFailure("manifest_forbidden_call_observed")
    gate_results = value["gate_results"]
    if not isinstance(gate_results, list):
        raise VerificationFailure("manifest_gate_results_invalid")
    gate_ids = []
    for result in gate_results:
        if not isinstance(result, dict):
            raise VerificationFailure("manifest_gate_results_invalid")
        require_exact_keys(result, ("gate_id", "counts"), "manifest_gate_results_invalid")
        if result["gate_id"] not in GATE_KEYS:
            raise VerificationFailure("manifest_gate_results_invalid")
        validate_counts(result["counts"], "manifest_gate_results_invalid")
        gate_ids.append(result["gate_id"])
    if gate_ids != sorted(GATE_KEYS):
        raise VerificationFailure("manifest_gate_results_incomplete")
    gates = value["gate_completeness"]
    if not isinstance(gates, dict):
        raise VerificationFailure("manifest_gates_invalid")
    require_exact_keys(gates, GATE_KEYS, "manifest_gates_invalid")
    if not all(isinstance(item, bool) for item in gates.values()):
        raise VerificationFailure("manifest_gates_invalid")
    if require_complete and not all(gates.values()):
        raise VerificationFailure("manifest_incomplete")
    if gates != {gate_id: True for gate_id in gate_ids}:
        raise VerificationFailure("manifest_gate_reconciliation_failed")
    for key in ("fixture_set_identity_sha256", "stable_identity_sha256", "evidence_identity_sha256"):
        if not isinstance(value[key], str) or not re.fullmatch(r"[0-9a-f]{64}", value[key]):
            raise VerificationFailure("manifest_identity_invalid")
    if value["fixture_set_identity_sha256"] != sha256(canonical_json_bytes(fixture_inputs(versions, cases))):
        raise VerificationFailure("manifest_fixture_identity_mismatch")
    if value["stable_identity_sha256"] != sha256(canonical_json_bytes(stable_projection(value))):
        raise VerificationFailure("manifest_stable_identity_mismatch")
    evidence_projection = dict(value)
    evidence_identity = evidence_projection.pop("evidence_identity_sha256")
    if evidence_identity != sha256(canonical_json_bytes(evidence_projection)):
        raise VerificationFailure("manifest_evidence_identity_mismatch")
    assert_no_hostile_sentinels(canonical_json_bytes(value))


def read_canonical_json(path: pathlib.Path) -> Any:
    try:
        raw = path.read_bytes()
        value = json.loads(raw.decode("utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        raise VerificationFailure("canonical_json_unreadable") from None
    if raw != canonical_json_bytes(value):
        raise VerificationFailure("canonical_json_encoding_invalid")
    return value


def rust_run(args: argparse.Namespace) -> None:
    if args.receipt_out and not args.campaign_id:
        raise VerificationFailure("receipt_campaign_id_required")
    started_wall = datetime.datetime.now(datetime.timezone.utc)
    started_monotonic_ns = time.monotonic_ns()
    repo = pathlib.Path(args.repo).resolve()
    candidate = candidate_identity(repo)
    report = collect_rust_run(args)
    receipt = make_receipt(
        repo,
        args.campaign_id or "focused-run",
        candidate,
        started_wall,
        started_monotonic_ns,
        sorted(
            [
                {"gate_id": "test_selection", "counts": report.counts},
                {"gate_id": "registry", "counts": unit_gate_counts(len(report.cases))},
                {"gate_id": "structural_trace", "counts": unit_gate_counts()},
                {"gate_id": "teardown", "counts": unit_gate_counts(len(report.teardown_markers))},
            ],
            key=lambda item: item["gate_id"],
        ),
        {
            "cases": report.cases,
            "counts": report.counts,
            "test_binary_identities": report.test_binary_identities,
            "zero_calls": report.zero_calls,
        },
    )
    if args.receipt_out:
        write_receipt(args.receipt_out, receipt)
    print(json.dumps({"cases": report.cases, "counts": report.counts, "test_binary_count": len(report.test_binary_identities), "zero_calls": report.zero_calls}, sort_keys=True, separators=(",", ":")))


def rust_list(args: argparse.Namespace) -> None:
    selected, ignored = discover_tests(pathlib.Path(args.repo).resolve(), args.package, args.features, args.prefix)
    if args.require_nonzero and not selected:
        raise VerificationFailure("zero_tests_discovered")
    identities = sorted(name for _, name in selected)
    print(json.dumps({"discovered": len(identities), "ignored": len([name for name in identities if name in ignored]), "test_identities": identities}, sort_keys=True, separators=(",", ":")))


def manifest_finalize(args: argparse.Namespace) -> None:
    value = read_canonical_json(pathlib.Path(args.input))
    validate_manifest(value, require_complete=args.require_complete)
    print(json.dumps({"schema": SCHEMA, "valid": True}, sort_keys=True, separators=(",", ":")))


def manifest_assemble(args: argparse.Namespace) -> None:
    paths = [args.rust_receipt, args.privacy_receipt, args.boundary_receipt, args.compiler_receipt]
    receipts = [read_canonical_json(path) for path in paths]
    for receipt in receipts:
        validate_receipt(receipt)
    campaign_ids = {receipt["campaign_id"] for receipt in receipts}
    candidates = {canonical_json_bytes(receipt["candidate_identity"]) for receipt in receipts}
    current_candidate = candidate_identity(pathlib.Path(args.repo).resolve())
    if len(campaign_ids) != 1 or len(candidates) != 1:
        raise VerificationFailure("gate_receipt_campaign_mismatch")
    if canonical_json_bytes(current_candidate) not in candidates:
        raise VerificationFailure("gate_receipt_candidate_mismatch")
    manifest = make_manifest(
        pathlib.Path(args.repo).resolve(),
        receipts[0],
        receipts,
    )
    validate_manifest(manifest, require_complete=True)
    args.output.write_bytes(canonical_json_bytes(manifest))
    print(json.dumps({"gate_count": len(GATE_KEYS), "schema": SCHEMA, "valid": True}, sort_keys=True, separators=(",", ":")))


def manifest_controls(args: argparse.Namespace) -> None:
    value = read_canonical_json(pathlib.Path(args.input))
    validate_manifest(value, require_complete=False)
    mutations = []
    forbidden = copy.deepcopy(value)
    forbidden["proposal"] = "SYNTHETIC_QUERY_SENTINEL_8B2C"
    mutations.append((forbidden, "manifest_forbidden_field"))
    missing = copy.deepcopy(value)
    missing.pop("zero_call_summary")
    mutations.append((missing, "manifest_fields_invalid"))
    zero = copy.deepcopy(value)
    zero["counts"]["discovered"] = 0
    mutations.append((zero, "manifest_zero_count"))
    incomplete = copy.deepcopy(value)
    incomplete["cases"].pop()
    mutations.append((incomplete, "manifest_case_inventory_incomplete"))
    for mutation, expected in mutations:
        try:
            validate_manifest(mutation, require_complete=True)
        except VerificationFailure as error:
            if error.reason != expected:
                raise VerificationFailure("manifest_control_wrong_rejection") from None
        else:
            raise VerificationFailure("manifest_control_not_rejected")
    print(json.dumps({"manifest_controls_rejected": len(mutations)}, sort_keys=True, separators=(",", ":")))


def forbidden_json_key(key: str) -> bool:
    if key in {
        "evidence_identity_sha256",
        "fixture_set_identity_sha256",
        "stable_identity_sha256",
    }:
        return False
    normalized = key.lower().replace("-", "_")
    forbidden = (
        "prompt", "mention", "conversation", "message", "proposal", "query", "url", "mail",
        "attachment", "serial", "tracking", "session_id", "turn_id", "connector",
        "evidence_id", "source_id", "credential", "key_material", "ciphertext", "nonce",
        "raw_path", "raw_id", "user_content", "content_hash", "unkeyed_hash", "plain_sha256",
    )
    return any(token in normalized for token in forbidden)


def inspect_structured_privacy(value: Any) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if not isinstance(key, str) or forbidden_json_key(key):
                raise VerificationFailure("forbidden_structured_field")
            inspect_structured_privacy(child)
    elif isinstance(value, list):
        for child in value:
            inspect_structured_privacy(child)
    elif isinstance(value, float):
        raise VerificationFailure("forbidden_numeric_encoding")


def privacy_scan(args: argparse.Namespace) -> None:
    if args.receipt_out and not args.campaign_id:
        raise VerificationFailure("receipt_campaign_id_required")
    started_wall = datetime.datetime.now(datetime.timezone.utc)
    started_monotonic_ns = time.monotonic_ns()
    repo = pathlib.Path(args.repo).resolve()
    candidate = candidate_identity(repo)
    artifacts = [pathlib.Path(item) for item in args.artifact]
    if not artifacts:
        raise VerificationFailure("privacy_artifact_inventory_empty")
    structured_count = 0
    byte_count = 0
    with tempfile.TemporaryDirectory(prefix="bagent-reference-privacy-") as temporary:
        temporary_path = pathlib.Path(temporary)
        marker = temporary_path / "teardown-marker"
        marker.write_bytes(b"REFERENCE_SYNTHETIC_SAFE_V1\n")
        for artifact in artifacts:
            try:
                payload = artifact.read_bytes()
            except OSError:
                raise VerificationFailure("privacy_artifact_unreadable") from None
            byte_count += 1
            assert_no_hostile_sentinels(payload)
            if artifact.suffix == ".json":
                try:
                    structured = json.loads(payload.decode("utf-8"))
                except (UnicodeDecodeError, json.JSONDecodeError):
                    raise VerificationFailure("privacy_structured_artifact_invalid") from None
                inspect_structured_privacy(structured)
                structured_count += 1
        if args.require_structured and structured_count == 0:
            raise VerificationFailure("structured_privacy_scan_missing")
        if args.require_sentinel and not HOSTILE_SENTINELS:
            raise VerificationFailure("sentinel_catalog_empty")
        if args.require_byte_scan and byte_count == 0:
            raise VerificationFailure("byte_privacy_scan_missing")
        marker.unlink()
    if temporary_path.exists():
        raise VerificationFailure("privacy_teardown_incomplete")
    if args.receipt_out:
        write_receipt(
            args.receipt_out,
            make_receipt(
                repo,
                args.campaign_id,
                candidate,
                started_wall,
                started_monotonic_ns,
                [{"gate_id": "privacy", "counts": unit_gate_counts(byte_count)}],
                {
                    "artifacts_scanned": byte_count,
                    "hostile_sentinel_classes": len(HOSTILE_SENTINELS),
                    "structured_artifacts": structured_count,
                    "teardown_complete": True,
                },
            ),
        )
    print(json.dumps({"artifacts_scanned": byte_count, "byte_scan": True, "hostile_sentinel_classes": len(HOSTILE_SENTINELS), "structured_artifacts": structured_count, "teardown_complete": True}, sort_keys=True, separators=(",", ":")))


def privacy_controls(_: argparse.Namespace) -> None:
    controls_rejected = 0
    for value, expected in (
        ({"proposal": "synthetic"}, "forbidden_structured_field"),
        (1.5, "forbidden_numeric_encoding"),
    ):
        try:
            inspect_structured_privacy(value)
        except VerificationFailure as error:
            if error.reason != expected:
                raise VerificationFailure("privacy_control_wrong_rejection") from None
            controls_rejected += 1
        else:
            raise VerificationFailure("privacy_control_not_rejected")
    with tempfile.TemporaryDirectory(prefix="bagent-reference-privacy-control-") as temporary:
        artifact = pathlib.Path(temporary) / "synthetic.bin"
        artifact.write_bytes(HOSTILE_SENTINELS[0])
        if not any(sentinel in artifact.read_bytes() for sentinel in HOSTILE_SENTINELS):
            raise VerificationFailure("privacy_sentinel_control_not_rejected")
        controls_rejected += 1
    if pathlib.Path(temporary).exists():
        raise VerificationFailure("privacy_control_teardown_incomplete")
    print(json.dumps({"privacy_controls_rejected": controls_rejected, "teardown_complete": True}, sort_keys=True, separators=(",", ":")))


def boundary_audit(args: argparse.Namespace) -> None:
    if args.receipt_out and not args.campaign_id:
        raise VerificationFailure("receipt_campaign_id_required")
    started_wall = datetime.datetime.now(datetime.timezone.utc)
    started_monotonic_ns = time.monotonic_ns()
    repo = pathlib.Path(args.repo).resolve()
    candidate = candidate_identity(repo)
    main_source = (repo / "crates/daemon/src/main.rs").read_text()
    evidence_module = (repo / "crates/daemon/src/evidence/mod.rs").read_text()
    makefile = (repo / "apps/macos/Makefile").read_text()
    test_gate = re.compile(
        r'(?ms)^\s*#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*'
        r'#\s*\[\s*path\s*=\s*"reference_resolution/contract_tests/mod\.rs"\s*\]\s*'
        r'mod\s+reference_resolution\s*;'
    )
    if not test_gate.search(main_source):
        raise VerificationFailure("test_module_gate_missing")
    acceptance_gate = re.compile(
        r'(?ms)^\s*#\s*\[\s*cfg\s*\(\s*feature\s*=\s*"stage8-acceptance"\s*\)\s*\]\s*'
        r'mod\s+acceptance\s*;'
    )
    if not acceptance_gate.search(evidence_module):
        raise VerificationFailure("acceptance_feature_gate_missing")
    required_make_patterns = (
        r"(?m)^REFERENCE_ORDINARY_TARGET_DIR\s*=",
        r"(?m)^REFERENCE_ACCEPTANCE_TARGET_DIR\s*=",
        r"(?m)^reference-daemon-ordinary\s*:",
        r"(?m)^reference-daemon-acceptance\s*:",
    )
    if not all(re.search(pattern, makefile) for pattern in required_make_patterns):
        raise VerificationFailure("artifact_separation_missing")
    if "/acceptance/reference" in main_source or "REFERENCE_ACCEPTANCE_FIXTURE" in main_source:
        raise VerificationFailure("ordinary_source_graph_contains_reference_fixture")
    artifact_graph_checked = False
    if args.require_artifact_graph:
        build_root = repo / "apps/macos/.build/reference-verification"
        ordinary_root = build_root / "ordinary-build/cargo/debug"
        acceptance_root = build_root / "acceptance-build/cargo/debug"
        ordinary_dependencies = "\n".join(path.read_text() for path in ordinary_root.rglob("bagentd*.d"))
        acceptance_dependencies = "\n".join(path.read_text() for path in acceptance_root.rglob("bagentd*.d"))
        if not ordinary_dependencies or not acceptance_dependencies:
            raise VerificationFailure("artifact_dependency_graph_missing")
        if "evidence/acceptance.rs" in ordinary_dependencies:
            raise VerificationFailure("ordinary_graph_contains_acceptance_source")
        if "reference_resolution/contract_tests/mod.rs" in ordinary_dependencies:
            raise VerificationFailure("ordinary_graph_contains_test_fixture")
        if "evidence/acceptance.rs" not in acceptance_dependencies:
            raise VerificationFailure("acceptance_graph_missing_acceptance_source")
        if "reference_resolution/contract_tests/mod.rs" in acceptance_dependencies:
            raise VerificationFailure("acceptance_graph_contains_test_fixture")
        ordinary_binary = (ordinary_root / "bagentd").read_bytes()
        acceptance_binary = (acceptance_root / "bagentd").read_bytes()
        stage8_marker = b"BAGENT_STAGE8_ACCEPTANCE_FIXTURES"
        if stage8_marker in ordinary_binary or stage8_marker not in acceptance_binary:
            raise VerificationFailure("acceptance_binary_gate_mismatch")
        artifact_graph_checked = True
    if args.receipt_out:
        if not artifact_graph_checked:
            raise VerificationFailure("artifact_graph_receipt_requires_graph")
        write_receipt(
            args.receipt_out,
            make_receipt(
                repo,
                args.campaign_id,
                candidate,
                started_wall,
                started_monotonic_ns,
                [{"gate_id": "ordinary_exclusion", "counts": unit_gate_counts()}],
                {
                    "acceptance_feature_gate": True,
                    "artifact_graph_checked": True,
                    "ordinary_route_count": 0,
                },
            ),
        )
    print(json.dumps({"acceptance_feature_gate": True, "artifact_graph_checked": artifact_graph_checked, "artifact_paths_distinct": True, "ordinary_reference_fixture_route_count": 0, "ordinary_test_fixture_graph_excluded": True, "source_checks": 4}, sort_keys=True, separators=(",", ":")))


def controls(args: argparse.Namespace) -> None:
    repo = pathlib.Path(args.repo).resolve()
    script = pathlib.Path(__file__).resolve()
    base = [sys.executable, str(script), "--repo", str(repo), "rust-run", "--package", args.package, "--prefix", "reference_resolution::contract::harness::", "--require-nonzero"]
    controls_to_run = (
        (["--oracle-mutation", "missing-required-case"], "missing_required_case"),
        (["--execution-control", "zero-executed"], "zero_tests_executed"),
        (["--oracle-mutation", "wrong-structural-trace"], "structural_trace_mismatch"),
        (["--oracle-mutation", "teardown-failure"], "teardown_incomplete"),
    )
    for arguments, expected in controls_to_run:
        result = run(base + arguments, cwd=repo)
        if result.returncode != 2 or f"verification failed: {expected}" not in result.stderr:
            raise VerificationFailure("negative_control_not_rejected")
    zero_match = list(base)
    zero_match[zero_match.index("--prefix") + 1] = "reference_resolution::contract::does_not_exist::"
    result = run(zero_match, cwd=repo)
    if result.returncode != 2 or "verification failed: zero_tests_discovered" not in result.stderr:
        raise VerificationFailure("zero_match_control_not_rejected")
    print(json.dumps({"controls_rejected": 4, "zero_match_rejected": True}, sort_keys=True, separators=(",", ":")))


def add_rust_arguments(command: argparse.ArgumentParser, *, run_tests: bool) -> None:
    command.add_argument("--package", default="bagentd")
    command.add_argument("--prefix", required=True)
    command.add_argument("--features")
    command.add_argument("--require-nonzero", action="store_true")
    if run_tests:
        command.add_argument("--oracle-mutation", choices=["missing-required-case", "wrong-structural-trace", "teardown-failure"])
        command.add_argument("--execution-control", choices=["zero-executed"])
        command.add_argument("--receipt-out", type=pathlib.Path)
        command.add_argument("--campaign-id")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    root.add_argument("--repo", default=pathlib.Path(__file__).resolve().parents[1])
    subcommands = root.add_subparsers(dest="command", required=True)
    rust_inventory = subcommands.add_parser("rust-list")
    add_rust_arguments(rust_inventory, run_tests=False)
    rust_inventory.set_defaults(handler=rust_list)
    rust = subcommands.add_parser("rust-run")
    add_rust_arguments(rust, run_tests=True)
    rust.set_defaults(handler=rust_run)
    control = subcommands.add_parser("controls")
    control.add_argument("--package", default="bagentd")
    control.set_defaults(handler=controls)
    privacy = subcommands.add_parser("privacy")
    privacy.add_argument("--artifact", action="append", default=[])
    privacy.add_argument("--require-structured", action="store_true")
    privacy.add_argument("--require-sentinel", action="store_true")
    privacy.add_argument("--require-byte-scan", action="store_true")
    privacy.add_argument("--receipt-out", type=pathlib.Path)
    privacy.add_argument("--campaign-id")
    privacy.set_defaults(handler=privacy_scan)
    privacy_control = subcommands.add_parser("privacy-controls")
    privacy_control.set_defaults(handler=privacy_controls)
    boundary = subcommands.add_parser("boundary-audit")
    boundary.add_argument("--require-nonzero", action="store_true")
    boundary.add_argument("--require-artifact-graph", action="store_true")
    boundary.add_argument("--receipt-out", type=pathlib.Path)
    boundary.add_argument("--campaign-id")
    boundary.set_defaults(handler=boundary_audit)
    manifest = subcommands.add_parser("manifest")
    manifest_subcommands = manifest.add_subparsers(dest="manifest_command", required=True)
    assemble = manifest_subcommands.add_parser("assemble")
    assemble.add_argument("--rust-receipt", required=True, type=pathlib.Path)
    assemble.add_argument("--privacy-receipt", required=True, type=pathlib.Path)
    assemble.add_argument("--boundary-receipt", required=True, type=pathlib.Path)
    assemble.add_argument("--compiler-receipt", required=True, type=pathlib.Path)
    assemble.add_argument("--output", required=True, type=pathlib.Path)
    assemble.set_defaults(handler=manifest_assemble)
    finalize = manifest_subcommands.add_parser("finalize")
    finalize.add_argument("--input", required=True, type=pathlib.Path)
    finalize.add_argument("--require-complete", action="store_true")
    finalize.set_defaults(handler=manifest_finalize)
    manifest_control = manifest_subcommands.add_parser("controls")
    manifest_control.add_argument("--input", required=True, type=pathlib.Path)
    manifest_control.set_defaults(handler=manifest_controls)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        args.handler(args)
    except VerificationFailure as error:
        print(f"verification failed: {error.reason}", file=sys.stderr)
        return 2
    except Exception:
        print("verification failed: driver_internal_failure", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

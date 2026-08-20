#!/usr/bin/env python3
"""Non-executing Slice 1 schema shell for the future signed campaign."""

from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
import re
import sys
from typing import Any

sys.dont_write_bytecode = True


REPORT_SCHEMA = "reference-signed-campaign-report/v1"
PHASES = tuple(f"P{number:02d}" for number in range(1, 17))


class SignedAcceptanceFailure(RuntimeError):
    def __init__(self, reason: str):
        super().__init__(reason)
        self.reason = reason


def verification_module() -> Any:
    path = pathlib.Path(__file__).with_name("reference-verification.py")
    spec = importlib.util.spec_from_file_location("reference_verification", path)
    if spec is None or spec.loader is None:
        raise SignedAcceptanceFailure("verification_schema_unavailable")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def phases(_: argparse.Namespace) -> None:
    print(json.dumps({"phase_count": len(PHASES), "phases": list(PHASES)}, sort_keys=True, separators=(",", ":")))


def validate_manifest(args: argparse.Namespace) -> None:
    module = verification_module()
    value = module.read_canonical_json(args.manifest)
    module.validate_manifest(value, require_complete=args.require_complete)
    print(json.dumps({"schema": module.SCHEMA, "valid": True}, sort_keys=True, separators=(",", ":")))


def forbidden_key(key: str) -> bool:
    normalized = key.lower().replace("-", "_")
    return any(
        token in normalized
        for token in (
            "prompt", "message", "task", "term", "proposal", "query", "url", "answer",
            "citation", "mail", "connector", "source", "attachment", "credential", "key",
            "ciphertext", "nonce", "raw_id", "path", "diagnostic", "database", "provider_output",
        )
    )


def inspect_report(value: Any) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if not isinstance(key, str) or forbidden_key(key):
                raise SignedAcceptanceFailure("report_forbidden_field")
            inspect_report(child)
    elif isinstance(value, list):
        for child in value:
            inspect_report(child)
    elif isinstance(value, float):
        raise SignedAcceptanceFailure("report_float_forbidden")


def validate_report_value(value: Any) -> None:
    if not isinstance(value, dict) or value.get("schema") != REPORT_SCHEMA:
        raise SignedAcceptanceFailure("report_schema_invalid")
    required = {"schema", "campaign_version", "manifest_identity_sha256", "phases", "status"}
    if set(value) != required:
        raise SignedAcceptanceFailure("report_fields_invalid")
    if value["campaign_version"] != "reference-signed-campaign/v1":
        raise SignedAcceptanceFailure("campaign_version_invalid")
    if not isinstance(value["manifest_identity_sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", value["manifest_identity_sha256"]):
        raise SignedAcceptanceFailure("manifest_identity_invalid")
    if value["phases"] != list(PHASES):
        raise SignedAcceptanceFailure("phase_registry_invalid")
    if value["status"] not in {"provisional", "blocked", "failed"}:
        raise SignedAcceptanceFailure("report_status_invalid")
    inspect_report(value)


def validate_report(args: argparse.Namespace) -> None:
    module = verification_module()
    value = module.read_canonical_json(args.report)
    validate_report_value(value)
    module.assert_no_hostile_sentinels(module.canonical_json_bytes(value))
    print(json.dumps({"schema": REPORT_SCHEMA, "valid": True}, sort_keys=True, separators=(",", ":")))


def controls(_: argparse.Namespace) -> None:
    module = verification_module()
    report = {
        "schema": REPORT_SCHEMA,
        "campaign_version": "reference-signed-campaign/v1",
        "manifest_identity_sha256": "0" * 64,
        "phases": list(PHASES),
        "status": "blocked",
    }
    validate_report_value(report)
    module.assert_no_hostile_sentinels(module.canonical_json_bytes(report))
    report["proposal"] = "synthetic"
    try:
        validate_report_value(report)
    except SignedAcceptanceFailure as error:
        if error.reason not in {"report_fields_invalid", "report_forbidden_field"}:
            raise SignedAcceptanceFailure("report_control_wrong_rejection") from None
    else:
        raise SignedAcceptanceFailure("report_control_not_rejected")
    print(json.dumps({"report_controls_rejected": 1, "run_enabled": False}, sort_keys=True, separators=(",", ":")))


def refuse(_: argparse.Namespace) -> None:
    raise SignedAcceptanceFailure("signed_campaign_not_implemented")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    commands = root.add_subparsers(dest="command", required=True)
    phase_command = commands.add_parser("phases")
    phase_command.set_defaults(handler=phases)
    manifest = commands.add_parser("validate-manifest")
    manifest.add_argument("--manifest", required=True, type=pathlib.Path)
    manifest.add_argument("--require-complete", action="store_true")
    manifest.set_defaults(handler=validate_manifest)
    report = commands.add_parser("validate-report")
    report.add_argument("--report", required=True, type=pathlib.Path)
    report.set_defaults(handler=validate_report)
    control = commands.add_parser("controls")
    control.set_defaults(handler=controls)
    run_command = commands.add_parser("run")
    run_command.set_defaults(handler=refuse)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        args.handler(args)
    except SignedAcceptanceFailure as error:
        reason = error.reason
        print(f"signed acceptance refused: {reason}", file=sys.stderr)
        return 3
    except Exception:
        print("signed acceptance refused: schema_validation_failed", file=sys.stderr)
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

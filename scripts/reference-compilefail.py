#!/usr/bin/env python3
"""Run cfg-selected, dependency-free compiler-fail reference fixtures."""

from __future__ import annotations

import argparse
import datetime
import hashlib
import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass


@dataclass(frozen=True)
class Fixture:
    fixture_id: str
    cfg_value: str
    source: str
    source_marker: str
    diagnostic_code: str | None


sys.dont_write_bytecode = True

FIXTURES = (
    Fixture(
        fixture_id="synthetic_type_mismatch",
        cfg_value="synthetic_type_mismatch",
        source="crates/daemon/src/reference_resolution/contract_tests/mod.rs",
        source_marker='const SYNTHETIC_TYPE_MISMATCH: u8 = "closed-enum-required";',
        diagnostic_code="E0308",
    ),
    Fixture(
        fixture_id="producer_constructor_privacy",
        cfg_value="producer_constructor_privacy",
        source="crates/daemon/src/reference_resolution/contract_tests/mod.rs",
        source_marker="CanonicalWebArtifact::private_constructor_for_internal();",
        diagnostic_code="E0624",
    ),
    Fixture(
        fixture_id="producer_witness_reuse",
        cfg_value="producer_witness_reuse",
        source="crates/daemon/src/reference_resolution/contract_tests/mod.rs",
        source_marker="        let witness_reused = witness;",
        diagnostic_code="E0382",
    ),
    Fixture(
        fixture_id="producer_witness_clone",
        cfg_value="producer_witness_clone",
        source="crates/daemon/src/reference_resolution/contract_tests/mod.rs",
        source_marker="        let _ = witness.clone();",
        diagnostic_code="E0599",
    ),
    Fixture(
        fixture_id="producer_cross_producer",
        cfg_value="producer_cross_producer",
        source="crates/daemon/src/reference_resolution/contract_tests/mod.rs",
        source_marker="            deterministic_witness,",
        diagnostic_code="E0308",
    ),
    Fixture("slice9_raw_string_search", "slice9_raw_string_search", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        typed(value); // Slice 9 fixture: raw String cannot call typed search.", "E0308"),
    Fixture("slice9_str_search", "slice9_str_search", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        typed(value); // Slice 9 fixture: &str cannot call typed search.", "E0308"),
    Fixture("slice9_raw_url_direct_fetch", "slice9_raw_url_direct_fetch", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        typed(value); // Slice 9 fixture: raw Url cannot call direct fetch.", "E0308"),
    Fixture("slice9_web_candidate_fetch", "slice9_web_candidate_fetch", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        typed(value); // Slice 9 fixture: ordinary WebCandidate cannot fetch.", "E0308"),
    Fixture("slice9_permit_constructor", "slice9_permit_constructor", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        let _ = crate::reference_resolution::ProviderQueryPermit::new(); // Slice 9 fixture: permit constructor is private.", "E0599"),
    Fixture("slice9_authorized_constructor", "slice9_authorized_constructor", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        let _ = crate::reference_resolution::AuthorizedSearch::new(); // Slice 9 fixture: authorized constructor is private.", "E0599"),
    Fixture("slice9_capability_clone", "slice9_capability_clone", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        let _ = value.clone(); // Slice 9 fixture: capability cannot clone.", "E0599"),
    Fixture("slice9_capability_serialize", "slice9_capability_serialize", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        let _ = serde_json::to_string(&value); // Slice 9 fixture: capability cannot serialize.", "E0277"),
    Fixture("slice9_capability_deserialize", "slice9_capability_deserialize", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        let _ = serde_json::from_str::<Permit>(\"{}\"); // Slice 9 fixture: capability cannot deserialize.", "E0277"),
    Fixture("slice9_capability_display", "slice9_capability_display", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        let _ = format!(\"{value}\"); // Slice 9 fixture: capability cannot display.", "E0277"),
    Fixture("slice9_capability_copy", "slice9_capability_copy", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        let _copy_again = value; // Slice 9 fixture: capability cannot copy.", "E0382"),
    Fixture("slice9_moved_operation", "slice9_moved_operation", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        typed(value); // Slice 9 fixture: moved authorized operation cannot be reused.", "E0382"),
    Fixture("slice9_search_as_fetch", "slice9_search_as_fetch", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        typed(value); // Slice 9 fixture: search authorization cannot fetch.", "E0308"),
    Fixture("slice9_candidate_forge", "slice9_candidate_forge", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        let _ = crate::reference_resolution::SealedDiscoveredCandidate {}; // Slice 9 fixture: candidate cannot be forged.", None),
    Fixture("slice9_direct_as_search", "slice9_direct_as_search", "crates/daemon/src/reference_resolution/contract_tests/mod.rs", "        typed(value); // Slice 9 fixture: direct authorization cannot become search.", "E0308"),
)
class CompileFailFailure(RuntimeError):
    def __init__(self, reason: str):
        super().__init__(reason)
        self.reason = reason


def verification_module():
    path = pathlib.Path(__file__).with_name("reference-verification.py")
    spec = importlib.util.spec_from_file_location("reference_verification", path)
    if spec is None or spec.loader is None:
        raise CompileFailFailure("privacy_catalog_unavailable")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


VERIFICATION = verification_module()
HOSTILE_SENTINELS = VERIFICATION.HOSTILE_SENTINELS


def canonical_json_bytes(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, allow_nan=False, sort_keys=True, separators=(",", ":")).encode("utf-8") + b"\n"


def assert_private_bytes_absent(value: str) -> None:
    payload = value.encode("utf-8")
    if any(sentinel in payload for sentinel in HOSTILE_SENTINELS):
        raise CompileFailFailure("forbidden_sentinel_present")


def cargo_command(package: str, target_dir: pathlib.Path) -> list[str]:
    return [
        "cargo",
        "test",
        "-p",
        package,
        "--no-run",
        "--message-format=json",
        "--target-dir",
        str(target_dir),
    ]


def run_cargo(
    repo: pathlib.Path,
    package: str,
    target_dir: pathlib.Path,
    cfg_value: str | None,
) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    if cfg_value is not None:
        inherited = environment.get("RUSTFLAGS", "").strip()
        fixture_flag = f'--cfg=reference_compilefail_fixture="{cfg_value}"'
        environment["RUSTFLAGS"] = " ".join(item for item in (inherited, fixture_flag) if item)
    return subprocess.run(
        cargo_command(package, target_dir),
        cwd=repo,
        env=environment,
        text=True,
        capture_output=True,
        check=False,
    )


def expected_line(repo: pathlib.Path, fixture: Fixture) -> int:
    source = (repo / fixture.source).read_text().splitlines()
    matches = [index + 1 for index, line in enumerate(source) if fixture.source_marker in line]
    if len(matches) != 1:
        raise CompileFailFailure("fixture_span_marker_invalid")
    return matches[0]


def error_diagnostics(output: str) -> list[dict]:
    diagnostics = []
    for line in output.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("reason") != "compiler-message":
            continue
        diagnostic = message.get("message")
        if isinstance(diagnostic, dict) and diagnostic.get("level") == "error":
            diagnostics.append(diagnostic)
    return diagnostics


def fixture_matches(
    repo: pathlib.Path, fixture: Fixture, diagnostic: dict, line: int
) -> bool:
    code = diagnostic.get("code")
    if fixture.diagnostic_code is None:
        if code is not None:
            return False
    elif not isinstance(code, dict) or code.get("code") != fixture.diagnostic_code:
        return False
    expected = (repo / fixture.source).resolve()
    for span in diagnostic.get("spans", []):
        file_name = span.get("file_name")
        if not isinstance(file_name, str):
            continue
        actual = pathlib.Path(file_name)
        if not actual.is_absolute():
            actual = repo / actual
        if actual.resolve() == expected and span.get("line_start") == line and span.get("is_primary"):
            return True
    return False


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[1])
    parser.add_argument("--package", default="bagentd")
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--fixture", action="append", default=[])
    parser.add_argument("--require-nonzero", action="store_true")
    parser.add_argument("--receipt-out", type=pathlib.Path)
    parser.add_argument("--campaign-id")
    args = parser.parse_args()
    selected = list(FIXTURES) if args.all else [item for item in FIXTURES if item.fixture_id in args.fixture]
    try:
        if args.receipt_out and not args.campaign_id:
            raise CompileFailFailure("receipt_campaign_id_required")
        started_wall = datetime.datetime.now(datetime.timezone.utc)
        started_monotonic_ns = time.monotonic_ns()
        if args.require_nonzero and not selected:
            raise CompileFailFailure("zero_fixtures_selected")
        unknown = set(args.fixture) - {item.fixture_id for item in FIXTURES}
        if unknown:
            raise CompileFailFailure("unknown_fixture")
        repo = args.repo.resolve()
        candidate = VERIFICATION.candidate_identity(repo)
        with tempfile.TemporaryDirectory(prefix="bagent-reference-compilefail-") as temporary:
            temporary_path = pathlib.Path(temporary)
            control = run_cargo(repo, args.package, temporary_path / "control", None)
            assert_private_bytes_absent(control.stdout + control.stderr)
            if control.returncode:
                raise CompileFailFailure("control_configuration_failed")
            results = []
            for fixture in selected:
                failed = run_cargo(repo, args.package, temporary_path / fixture.fixture_id, fixture.cfg_value)
                assert_private_bytes_absent(failed.stdout + failed.stderr)
                if failed.returncode == 0:
                    raise CompileFailFailure("invalid_fixture_compiled")
                diagnostics = error_diagnostics(failed.stdout)
                line = expected_line(repo, fixture)
                matching = [item for item in diagnostics if fixture_matches(repo, fixture, item, line)]
                if len(matching) != 1 or len(diagnostics) != 1:
                    raise CompileFailFailure("diagnostic_mismatch")
                results.append(
                    {
                        "diagnostic_class": fixture.diagnostic_code or "span_only",
                        "fixture_id": fixture.fixture_id,
                        "registered_span_matched": True,
                    }
                )
        if temporary_path.exists():
            raise CompileFailFailure("teardown_incomplete")
        if args.receipt_out:
            receipt = VERIFICATION.make_receipt(
                repo,
                args.campaign_id,
                candidate,
                started_wall,
                started_monotonic_ns,
                [{"gate_id": "compiler_fail", "counts": VERIFICATION.unit_gate_counts(len(results))}],
                {
                    "control_compiled": True,
                    "fixture_count": len(results),
                    "fixture_set_sha256": hashlib.sha256(canonical_json_bytes(results)).hexdigest(),
                    "teardown_complete": True,
                },
            )
            VERIFICATION.write_receipt(args.receipt_out, receipt)
        print(
            json.dumps(
                {"control_compiled": True, "fixture_count": len(results), "fixtures": results, "teardown_complete": True},
                sort_keys=True,
                separators=(",", ":"),
            )
        )
        return 0
    except CompileFailFailure as error:
        print(f"compile-fail verification failed: {error.reason}", file=sys.stderr)
        return 2
    except Exception:
        print("compile-fail verification failed: driver_internal_failure", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

python3 - "$repo_root" <<'PY'
from __future__ import annotations

import pathlib
import re
import subprocess
import sys

root = pathlib.Path(sys.argv[1]).resolve()
src = root / "crates/daemon/src"
swift_src = root / "apps/macos/Sources/bagent"
cutover = src / "cutover.rs"
migration = root / "crates/daemon/migrations/V27__stage8_canonical_cleanup.sql"

# Legacy identifiers are allowed only inside the explicit, transactional
# migration copy. Keep the allowance counted so a new compatibility route
# cannot hide inside cutover.rs.
cutover_legacy_sql_allowlist = {
    "UPDATE AUTOMATION_RUNS": 1,
    "UPDATE PENDING_APPROVALS": 1,
    "FROM AUTOMATION_RUNS": 2,
    "FROM PENDING_APPROVALS": 2,
}
work_coordinator_legacy_sql_allowlist = {
    "DROP TABLE IF EXISTS AUTOMATION_SESSION_PENDING_APPROVALS": 1,
}


def strip_cfg_test_items(text: str) -> str:
    """Remove cfg(test) items before scanning the production graph."""
    marker = re.compile(r"#\[cfg\(test\)\]")
    while True:
        match = marker.search(text)
        if match is None:
            return text
        start = match.start()
        brace = text.find("{", match.end())
        semi = text.find(";", match.end())
        if semi != -1 and (brace == -1 or semi < brace):
            text = text[:start] + text[semi + 1 :]
            continue
        if brace == -1:
            text = text[:start]
            continue
        depth = 0
        end = brace
        while end < len(text):
            if text[end] == "{":
                depth += 1
            elif text[end] == "}":
                depth -= 1
                if depth == 0:
                    end += 1
                    break
            end += 1
        text = text[:start] + text[end:]


# `reference_resolution/contract_tests/` is a whole-tree test module: it is
# declared `#[cfg(test)] #[path = "contract_tests/mod.rs"] mod contract;`, so the
# compiler never builds it into the daemon. The inline `#[cfg(test)]` stripper
# cannot see that from inside the files themselves, so exclude the tree here and
# assert the gating declaration still exists, or the exclusion is not earned.
CONTRACT_TESTS_DIR = src / "reference_resolution" / "contract_tests"
_reference_mod = (src / "reference_resolution" / "mod.rs").read_text()
if '#[cfg(test)]\n#[path = "contract_tests/mod.rs"]' not in _reference_mod:
    print("contract_tests is no longer cfg(test)-gated; refusing to skip it", file=sys.stderr)
    raise SystemExit(2)


def production_files() -> list[pathlib.Path]:
    rust = [
        path
        for path in sorted(src.rglob("*.rs"))
        if CONTRACT_TESTS_DIR not in path.parents
    ]
    return rust + sorted(swift_src.rglob("*.swift"))


def source_text(path: pathlib.Path) -> str:
    text = path.read_text()
    return strip_cfg_test_items(text) if path.suffix == ".rs" else text


forbidden = {
    "daemon-wide legacy event authority": re.compile(
        r"\blegacy_projection_tx\b|\bproject_legacy_event\b|route\s*\(\s*[\"']/?events"
    ),
    "removed evidence authority switch": re.compile(
        r"BAGENT_EVIDENCE_ORCHESTRATOR|EvidenceOrchestrator|evidence_orchestrator"
    ),
    "obsolete prompt/debug result injection": re.compile(
        r"PromptDebugRecord|PromptDebugMessage|prompt_debug|debug_conversation|"
        r"response_for_audit|append_prompt_debug|\bdebug_trace\b|"
        r"\bdebugTrace\b|debugPayload|prompt_trace_id"
    ),
    "obsolete automation event shim": re.compile(
        r"\bproject_automation_definition_event\b"
    ),
    "computed notch compatibility accessor": re.compile(
        r"(?:\b(?:private\s+|internal\s+|public\s+)?var\s+"
        r"(?:isThinking|isExpanded|chatSurfaceMode|toolStatus)\b|"
        r"\b(?:notchPresentation|viewModel|self)\.(?:isThinking|isExpanded)\b)"
    ),
    "obsolete lifecycle SQL": re.compile(
        r"\b(?:FROM|INTO|UPDATE|JOIN|TABLE|ALTER\s+TABLE|DROP\s+TABLE(?:\s+IF\s+EXISTS)?)\s+"
        r"(?:automation_runs|pending_approvals|sessions|chat_turns|chat_turns_fts|"
        r"automation_session_pending_approvals)\b",
        re.IGNORECASE,
    ),
}


def findings_for(path: pathlib.Path, text: str) -> list[str]:
    findings: list[str] = []
    for label, pattern in forbidden.items():
        for match in pattern.finditer(text):
            if label == "obsolete lifecycle SQL" and path in {
                cutover,
                src / "work_coordinator.rs",
            }:
                phrase = re.sub(r"\s+", " ", match.group(0)).upper()
                allowlist = (
                    cutover_legacy_sql_allowlist
                    if path == cutover
                    else work_coordinator_legacy_sql_allowlist
                )
                allowed_phrase = phrase in allowlist
                seen_before = sum(
                    1
                    for prior in forbidden[label].finditer(text, 0, match.start())
                    if re.sub(r"\s+", " ", prior.group(0)).upper() == phrase
                )
                if allowed_phrase and seen_before < allowlist[phrase]:
                    continue
            line = text.count("\n", 0, match.start()) + 1
            findings.append(f"{path.relative_to(root)}:{line}: {label}: {match.group(0)}")
    return findings


# Red capability: each forbidden category must be observable by the same
# detector used against production. A detector that cannot reject these seeds
# is not a qualification gate.
seed = """
let legacy_projection_tx = 1;
fn project_legacy_event() { }
router.route(\"/events\", get(events));
let switch_name = \"BAGENT_EVIDENCE_ORCHESTRATOR\";
struct EvidenceOrchestratorFlag;
struct PromptDebugRecord;
let response_for_audit = true;
let event = "debug_trace";
case .debugTrace;
let prompt_trace_id = "opaque";
struct Presentation { var isThinking: Bool; }
let old = \"SELECT * FROM automation_runs\";
fn project_automation_definition_event() { }
"""
seed_hits = findings_for(root / "__stage8_seed.rs", seed)
seed_labels = {hit.split(": ", 2)[1] for hit in seed_hits}
if seed_labels != set(forbidden):
    missing = sorted(set(forbidden) - seed_labels)
    print("A51 red capability: FAIL; detector missed: " + ", ".join(missing))
    raise SystemExit(1)
print(f"A51 red capability: PASS ({len(seed_hits)} seeded forbidden matches)")


findings: list[str] = []
for path in production_files():
    findings.extend(findings_for(path, source_text(path)))

cutover_text = source_text(cutover)
for phrase, expected_count in cutover_legacy_sql_allowlist.items():
    actual_count = sum(
        1
        for match in forbidden["obsolete lifecycle SQL"].finditer(cutover_text)
        if re.sub(r"\s+", " ", match.group(0)).upper() == phrase
    )
    if actual_count != expected_count:
        findings.append(
            f"{cutover.relative_to(root)}: explicit migration allowlist {phrase!r}: "
            f"expected {expected_count}, found {actual_count}"
        )

work_coordinator_text = source_text(src / "work_coordinator.rs")
for phrase, expected_count in work_coordinator_legacy_sql_allowlist.items():
    actual_count = sum(
        1
        for match in forbidden["obsolete lifecycle SQL"].finditer(work_coordinator_text)
        if re.sub(r"\s+", " ", match.group(0)).upper() == phrase
    )
    if actual_count != expected_count:
        findings.append(
            f"{(src / 'work_coordinator.rs').relative_to(root)}: explicit cleanup allowlist "
            f"{phrase!r}: expected {expected_count}, found {actual_count}"
        )

required = {
    "forward migration": migration.exists()
    and "stage8_cleanup_state" in migration.read_text()
    and "schema_generation" in migration.read_text(),
    "explicit old-table removal": all(
        f'"{table}"' in cutover.read_text()
        for table in (
            "automation_runs",
            "pending_approvals",
            "sessions",
            "chat_turns",
            "chat_turns_fts",
        )
    ),
    "canonical run record table": any(
        "automation_run_records" in source_text(path) for path in production_files()
    ),
    "canonical approval projection": "work_approval_requests" in (src / "main.rs").read_text(),
    "canonical interaction mode": "NotchInteractionMode" in (swift_src / "NotchProjection.swift").read_text(),
    "canonical foreground activity": "hasActiveForegroundWork" in (swift_src / "NotchProjection.swift").read_text(),
    "stage 8 finalizer": "finalize_stage8_cleanup" in cutover.read_text(),
    "canonical legacy record preservation": "legacy_run_records" in cutover.read_text(),
}
missing = sorted(name for name, present in required.items() if not present)
if missing:
    print("A51 missing canonical edge(s): " + ", ".join(missing))
    raise SystemExit(1)

if findings:
    for finding in findings:
        print("A51 forbidden: " + finding)
    print(f"A51 production inventory: FAIL ({len(findings)} findings)")
    raise SystemExit(1)

print(
    "A51 cutover migration allowlist: PASS "
    f"({sum(cutover_legacy_sql_allowlist.values())} counted legacy SQL matches; "
    f"{sum(work_coordinator_legacy_sql_allowlist.values())} bootstrap cleanup match)"
)


checks = [
    ("work authority", root / "scripts/acceptance/work-authority.sh", []),
    ("model runtime authority", root / "scripts/acceptance/model-runtime-authority.sh", []),
    ("settings authority", root / "scripts/acceptance/settings-authority.sh", [str(root)]),
    ("notch mode authority", root / "scripts/acceptance/notch-mode-authority.sh", []),
]
for label, command, args in checks:
    result = subprocess.run(
        ["bash", str(command), *args],
        cwd=root,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    if result.returncode != 0:
        print(result.stdout, end="")
        print(f"A51 authority subgate: FAIL ({label})")
        raise SystemExit(1)
    print(f"A51 authority subgate: PASS ({label})")

print(f"A51 production inventory: PASS (0 findings; {len(required)} canonical assertions)")
print(f"A51 evidence metrics: case_count={len(seed_hits)} assertion_count={len(required)}")
PY

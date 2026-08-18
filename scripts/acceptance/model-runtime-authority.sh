#!/bin/bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)

python3 - "$repo_root" <<'PY'
from __future__ import annotations

import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
allowed_adapter = root / "crates/daemon/src/model_runtime.rs"
allowed_connector = root / "crates/connectors/basert/src/lib.rs"

rust_patterns = {
    "BaseRtClient outside the production adapter": r"\bBaseRtClient\b",
    "direct model load": r"\.load_model\s*\(",
    "direct model unload": r"\.unload_model\s*\(",
    "direct completion": r"\.(?:chat_stream(?:_with_tools)?|chat_complete_(?:json_)?bounded|generate_raw|generate_json)\s*\(",
    "direct readiness inspection": r"\.(?:model_readiness|inspect_models)\s*\(",
    "direct poison or fault reset": r"\.(?:clear_runtime_fault|set_runtime_fault|mark_poisoned|poison)\s*\(",
    "direct runtime restart": r"(?:restart_managed_basert|\.restart_runtime\s*\()",
    "synthesis-only runtime manager": r"\bModelRuntimeManager\b|\bSynthesisModelClient\b",
    "backup-model synthesis": r"ensure_fallback|fallback_model|SYNTHESIS_FALLBACK",
    "caller-owned lifecycle guard": r"RwLock\s*<\s*\(\s*\)\s*>",
}

swift_patterns = {
    "Swift BaseRT lifecycle caller": r"ensureBaseRTRunning|BaseRTLaunchAgent\.label\)\]",
    "Swift direct BaseRT request": r"(?:127\.0\.0\.1|\[::1\]):8082/(?:health|v1/models(?:/(?:load|unload))?|v1/chat/completions)",
    "Swift direct BaseRT service mutation": r"(?:launchctl[^\n]*com\.bagent\.basert|com\.bagent\.basert[^\n]*launchctl)",
}


def strip_rust_tests(text: str) -> str:
    lines = text.splitlines(keepends=True)
    kept: list[str] = []
    index = 0
    while index < len(lines):
        if not re.match(r"^\s*#\[cfg\(test\)\]\s*$", lines[index]):
            kept.append(lines[index])
            index += 1
            continue
        index += 1
        while index < len(lines) and (
            not lines[index].strip() or re.match(r"^\s*#\[", lines[index])
        ):
            index += 1
        braces = 0
        saw_brace = False
        while index < len(lines):
            line = lines[index]
            braces += line.count("{") - line.count("}")
            saw_brace = saw_brace or "{" in line
            index += 1
            if (saw_brace and braces == 0) or (not saw_brace and ";" in line):
                break
    return "".join(kept)


def findings_for(path: pathlib.Path, text: str) -> list[str]:
    patterns = swift_patterns if path.suffix == ".swift" else rust_patterns
    findings: list[str] = []
    for label, pattern in patterns.items():
        for match in re.finditer(pattern, text):
            line = text.count("\n", 0, match.start()) + 1
            findings.append(f"{path.relative_to(root)}:{line}: {label}: {match.group(0)}")
    return findings


# Red-capability check: the detector must reject seeded Rust and Swift callers,
# a fallback manager, a direct restart/unload, and a duplicate lifecycle guard.
seed = """
struct ModelRuntimeManager { guard: RwLock<()> }
fn forbidden(client: BaseRtClient) {
    client.load_model("x");
    client.unload_model("x");
    client.restart_runtime();
    client.chat_complete_bounded("x", vec![], 0.0, 1);
    client.model_readiness("x");
    client.clear_runtime_fault();
    ensure_fallback();
}
"""
seed_swift = "func forbidden() { ensureBaseRTRunning(); let x = \"http://127.0.0.1:8082/v1/models/load\"; launchctl(\"kickstart com.bagent.basert\") }"
seed_hits = findings_for(root / "seed.rs", seed) + findings_for(root / "seed.swift", seed_swift)
required_seed_labels = {
    "BaseRtClient outside the production adapter",
    "direct model load",
    "direct model unload",
    "direct completion",
    "direct readiness inspection",
    "direct poison or fault reset",
    "direct runtime restart",
    "synthesis-only runtime manager",
    "backup-model synthesis",
    "caller-owned lifecycle guard",
    "Swift BaseRT lifecycle caller",
    "Swift direct BaseRT request",
    "Swift direct BaseRT service mutation",
}
seen_seed_labels = {hit.split(": ", 2)[1] for hit in seed_hits}
missing_seed = sorted(required_seed_labels - seen_seed_labels)
if missing_seed:
    print("A17 FAIL: detector did not reject seeded forbidden paths:", ", ".join(missing_seed))
    raise SystemExit(1)

findings: list[str] = []
for path in sorted((root / "crates").rglob("*.rs")):
    if path in {allowed_adapter, allowed_connector} or "src" not in path.relative_to(root / "crates").parts:
        continue
    findings.extend(findings_for(path, strip_rust_tests(path.read_text())))
for path in sorted((root / "apps/macos/Sources").rglob("*.swift")):
    findings.extend(findings_for(path, path.read_text()))

adapter_text = allowed_adapter.read_text() if allowed_adapter.exists() else ""
required_adapter_operations = [
    r"\bBaseRtClient\b",
    r"\.load_model\s*\(",
    r"\.unload_model\s*\(",
    r"\.model_readiness\s*\(",
    r"\.inspect_models\s*\(",
    r"\.chat_stream_with_tools\s*\(",
    r"\.chat_complete_bounded\s*\(",
    r"\.chat_complete_json_bounded\s*\(",
    r"\.clear_runtime_fault\s*\(",
    r"\brestart_managed\s*\(",
]
missing_adapter = [pattern for pattern in required_adapter_operations if not re.search(pattern, adapter_text)]

if findings or missing_adapter:
    print(f"A17 seeded detector: PASS ({len(seed_hits)} forbidden matches)")
    for finding in findings:
        print(f"A17 forbidden: {finding}")
    for pattern in missing_adapter:
        print(f"A17 missing production-adapter operation: {pattern}")
    print(f"A17 FAIL: {len(findings)} forbidden production match(es), {len(missing_adapter)} missing adapter operation(s)")
    raise SystemExit(1)

print(f"A17 seeded detector: PASS ({len(seed_hits)} forbidden matches)")
print("A17 production graph: PASS (0 forbidden matches; sole BaseRT lifecycle/completion adapter)")
PY

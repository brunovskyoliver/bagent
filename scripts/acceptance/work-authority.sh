#!/bin/bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)

python3 - "$repo_root" <<'PY'
from __future__ import annotations
import pathlib, re, sys

root = pathlib.Path(sys.argv[1])

patterns = {
    "typed-origin adapter": r"\b(?:TypedOrigin|TypedModelRuntime)\b",
    "legacy synthetic Work identity": r"legacy:\{?session_id|legacy:screen-intent",
    "semaphore admission authority": r"\brun_slots\b|Semaphore::new\(",
    "in-memory approval authority": r"pending_approvals\s*:\s*Arc|oneshot::Sender\s*<\s*bool",
    "startup approval denial": r"approvals_denied_on_restart|recover_on_startup",
    "ad-hoc event authority": r"\bpublish_event\b|\bevents_tx\b|\bpublish_automation_event\b",
    "direct chat stop authority": r'route\s*\(\s*"/chat/(?:stop|cancel)"|fn\s+(?:stop|cancel)_chat',
    "duplicate Work identity": r"model_runtime\.rs[^\n]*pub struct WorkIdentity",
}

seed = '''
let run_slots = Semaphore::new(MAX_CONCURRENT_RUNS);
let adapter = TypedModelRuntime::new(runtime, TypedOrigin::Foreground, WorkIdentity::new("legacy:{session_id}"));
pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;
recover_on_startup(); publish_event(json); events_tx.send(json);
route("/chat/stop", post(stop_chat));
model_runtime.rs pub struct WorkIdentity(String);
"UPDATE works SET state='completed'";
ModelDemand::automation(work, model);
'''
seed_hits = {label for label, pattern in patterns.items() if re.search(pattern, seed)}
missing_seed = set(patterns) - seed_hits
if missing_seed:
    print("A26 FAIL: detector missed seeded categories: " + ", ".join(sorted(missing_seed)))
    raise SystemExit(1)
if not re.search(r'(?is)"(?:INSERT\s+(?:OR\s+\w+\s+)?INTO|UPDATE|DELETE\s+FROM)\s+work(?:s|_[a-z_]+)', seed):
    print("A26 FAIL: detector missed seeded direct canonical Work SQL writer")
    raise SystemExit(1)
if not re.search(r"ModelDemand::(?:foreground|automation)\s*\(", seed):
    print("A26 FAIL: detector missed seeded caller-owned model demand")
    raise SystemExit(1)

def function_bodies(text: str):
    """Yield (name, brace-matched body) for every fn in `text`. Heuristic
    (regex + brace counting, no real parser) but sufficient to catch a
    function that calls `.admit(` without any `.release_slot(` anywhere in
    its own body — the shape of a capacity-slot leak on an early return."""
    for match in re.finditer(r"(?:async\s+)?fn\s+(\w+)", text):
        brace_start = text.find("{", match.end())
        if brace_start == -1:
            continue
        depth = 0
        i = brace_start
        while i < len(text):
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        yield match.group(1), text[brace_start : i + 1]


admit_release_seed_leaky = "async fn leaky(state: &AppState, work: WorkIdentity) { state.work_authority.admit(work).await; return; }"
admit_release_seed_hits = [
    name
    for name, body in function_bodies(admit_release_seed_leaky)
    if "work_authority.admit(" in body and ".release_slot(" not in body
]
if not admit_release_seed_hits:
    print("A26 FAIL: detector missed seeded admit-without-release capacity leak")
    raise SystemExit(1)


def strip_tests(text: str) -> str:
    lines = text.splitlines(keepends=True); out=[]; i=0
    while i < len(lines):
        if not re.match(r"^\s*#\[cfg\(test\)\]\s*$", lines[i]):
            out.append(lines[i]); i += 1; continue
        i += 1
        while i < len(lines) and (not lines[i].strip() or re.match(r"^\s*#\[", lines[i])): i += 1
        depth=0; opened=False
        while i < len(lines):
            depth += lines[i].count("{") - lines[i].count("}"); opened |= "{" in lines[i]; i += 1
            if (opened and depth == 0) or (not opened and ";" in lines[i-1]): break
    return "".join(out)

production_paths = sorted((root / "crates/daemon/src").rglob("*.rs"))
findings=[]
for path in production_paths:
    text = strip_tests(path.read_text())
    synthetic = f"{path.name}\n{text}"
    for label, pattern in patterns.items():
        for match in re.finditer(pattern, synthetic):
            line = synthetic.count("\n", 0, match.start()) + 1
            findings.append(f"{path.relative_to(root)}:{line}: {label}: {match.group(0)}")
    for name, body in function_bodies(text):
        if "work_authority.admit(" in body and ".release_slot(" not in body:
            line = text.count("\n", 0, text.find(body)) + 1
            findings.append(
                f"{path.relative_to(root)}:{line}: fn {name}: calls work_authority.admit( "
                "without any .release_slot( in the same function body (capacity-slot leak "
                "risk on an early-return path)"
            )

allowed_work_writers = {
    root / "crates/daemon/src/work_coordinator.rs",
    root / "crates/daemon/src/cutover.rs",
}
for path in sorted((root / "crates/daemon/src").rglob("*.rs")):
    text = strip_tests(path.read_text())
    if path not in allowed_work_writers:
        for match in re.finditer(r'(?is)"(?:INSERT\s+(?:OR\s+\w+\s+)?INTO|UPDATE|DELETE\s+FROM)\s+work(?:s|_[a-z_]+)', text):
            line = text.count("\n", 0, match.start()) + 1
            findings.append(f"{path.relative_to(root)}:{line}: direct canonical Work SQL writer")
    if path.name != "model_runtime.rs":
        for match in re.finditer(r"ModelDemand::(?:foreground|automation)\s*\(", text):
            line = text.count("\n", 0, match.start()) + 1
            findings.append(f"{path.relative_to(root)}:{line}: caller-owned model demand")

main = strip_tests((root / "crates/daemon/src/main.rs").read_text())
automation = strip_tests((root / "crates/daemon/src/automations_api.rs").read_text())
model_runtime = strip_tests((root / "crates/daemon/src/model_runtime.rs").read_text())
required = {
    "Conversation admission": "submit_conversation(" in main,
    "Automation admission": "submit_automation(" in automation,
    "durable approval request": ".request_approval(" in main,
    "durable approval decision": ".resolve_approval(" in main,
    "canonical Model Runtime Work identity": "pub use crate::work_coordinator::WorkIdentity" in model_runtime,
    "ordered outbox projection": ".coordinator().events(" in main,
}
missing = [name for name, present in required.items() if not present]
if findings or missing:
    print(f"A26 seeded detector: PASS ({len(seed_hits) + 3} forbidden categories)")
    for finding in findings: print("A26 forbidden: " + finding)
    for item in missing: print("A26 missing authority edge: " + item)
    print(f"A26 FAIL: {len(findings)} forbidden matches, {len(missing)} missing edges")
    raise SystemExit(1)

print(f"A26 seeded detector: PASS ({len(seed_hits) + 3} forbidden categories)")
print("A26 production graph: PASS (zero forbidden authorities; canonical Work command/event path present)")
PY

#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)

python3 - "$root" <<'PY'
import pathlib
import re
import sys
import urllib.parse

root = pathlib.Path(sys.argv[1]).resolve()
pattern = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
checked = 0
failures = []

for source in sorted(root.rglob("*.md")):
    if any(part in {".git", ".build", "target"} for part in source.parts):
        continue
    text = source.read_text(encoding="utf-8")
    for raw_target in pattern.findall(text):
        target = raw_target.strip().strip("<>").split("#", 1)[0]
        if not target or re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:", target):
            continue
        target = urllib.parse.unquote(target)
        candidate = (source.parent / target).resolve() if not target.startswith("/") else (root / target.lstrip("/")).resolve()
        checked += 1
        try:
            candidate.relative_to(root)
        except ValueError:
            failures.append(f"{source.relative_to(root)} -> {raw_target} (outside repository)")
            continue
        if not candidate.exists():
            failures.append(f"{source.relative_to(root)} -> {raw_target}")

if failures:
    print("documentation links: FAIL", file=sys.stderr)
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    raise SystemExit(1)

if checked == 0:
    print("documentation links: FAIL (zero repository-relative links)", file=sys.stderr)
    raise SystemExit(1)

print(f"documentation links: PASS ({checked} repository-relative links checked)")
PY

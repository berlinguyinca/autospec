#!/usr/bin/env bash
set -eu
root="${1:-.}"
python3 - "$root" <<'PY'
import json, pathlib, re, sys
root = pathlib.Path(sys.argv[1]).resolve()
items, todo = [], []
for p in root.rglob("*"):
    if not p.is_file() or ".git" in p.parts or ".autospec" in p.parts: continue
    if p.suffix not in {".sh", ".py", ".rs", ".ts", ".js", ".md"}: continue
    try: text = p.read_text(errors="ignore")
    except OSError: continue
    for n, line in enumerate(text.splitlines(), 1):
        markers = ("T" + "ODO", "FIX" + "ME", "X" + "XX")
        if any(re.search(r"\b" + re.escape(marker) + r"\b", line) for marker in markers): todo.append((p.relative_to(root).as_posix(), n))
if todo:
    path, line = todo[0]
    items.append({"title": f"Resolve deferred work at {path}:{line}", "severity":"stability", "score":0.82, "verification":"unverified", "evidence":f"{path}:{line}"})
if (root / "docs/roadmap.md").exists():
    items.append({"title":"Close the next documented roadmap gap", "severity":"feature", "score":0.68, "verification":"unverified", "evidence":"docs/roadmap.md"})
items.append({"title":"Expand regression coverage for autonomous discovery", "severity":"stability", "score":0.74, "verification":"unverified", "evidence":"tests/"})
items.sort(key=lambda x: x["score"], reverse=True)
print(json.dumps({"candidates":items[:5],"source":"repository-signals","verified":False}))
PY

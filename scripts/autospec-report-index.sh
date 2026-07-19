#!/usr/bin/env bash
# scripts/autospec-report-index.sh — index generated Autospec reports.

set -eu
usage() { echo "Usage: autospec-report-index.sh [--repo-root DIR]"; }
die() { printf 'autospec-report-index: %s\n' "$*" >&2; exit 2; }
REPO_ROOT="$(pwd)"
while [ "$#" -gt 0 ]; do
    case "$1" in --repo-root) REPO_ROOT="$2"; shift 2 ;; -h|--help) usage; exit 0 ;; *) die "unknown arg: $1" ;; esac
done
[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" <<'PY'
import json, os, sys, time
root = os.path.realpath(sys.argv[1]); reports = os.path.join(root, ".autospec", "reports")
os.makedirs(reports, exist_ok=True)
items = []
for name in sorted(os.listdir(reports)):
    if not (name.endswith(".md") or name.endswith(".json")) or name in {"report-index.json"}:
        continue
    path = os.path.join(reports, name)
    summary, status = "", "unknown"
    if name.endswith(".json"):
        try:
            data = json.load(open(path, encoding="utf-8"))
            status = str(data.get("verdict") or data.get("status") or data.get("overall_status") or "unknown")
        except Exception:
            status = "invalid_json"
    else:
        for line in open(path, encoding="utf-8", errors="ignore"):
            if line.strip() and not line.startswith("#"):
                summary = line.strip()[:140]; break
    items.append({"path": f".autospec/reports/{name}", "generated_time": int(os.path.getmtime(path)), "summary": summary, "verdict_or_status": status, "next_recommended_command": "bash scripts/autospec-mvp-status.sh"})
report = {"schema": 1, "reports": items}
json.dump(report, open(os.path.join(reports, "report-index.json"), "w", encoding="utf-8"), indent=2, sort_keys=True); open(os.path.join(reports, "report-index.json"), "a").write("\n")
rows = "\n".join(f"| `{i['path']}` | {i['verdict_or_status']} | {i['summary']} | `{i['next_recommended_command']}` |" for i in items)
open(os.path.join(reports, "REPORT_INDEX.md"), "w", encoding="utf-8").write("\n".join(["# Autospec Report Index", "", "## Latest Reports", "", "| Report | Status | Summary | Next |", "| --- | --- | --- | --- |", rows or "| none | unknown | none | none |", ""]) )
print("report index: wrote reports")
PY

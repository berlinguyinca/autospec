#!/usr/bin/env bash
# scripts/autospec-clean-generated-reports.sh — clean generated report artifacts only.

set -eu
usage() { echo "Usage: autospec-clean-generated-reports.sh [--repo-root DIR] [--dry-run|--confirm]"; }
die() { printf 'autospec-clean-generated-reports: %s\n' "$*" >&2; exit 2; }
REPO_ROOT="$(pwd)"; CONFIRM=0
while [ "$#" -gt 0 ]; do
    case "$1" in --repo-root) REPO_ROOT="$2"; shift 2 ;; --dry-run) CONFIRM=0; shift ;; --confirm) CONFIRM=1; shift ;; -h|--help) usage; exit 0 ;; *) die "unknown arg: $1" ;; esac
done
[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$CONFIRM" <<'PY'
import json, os, sys
root = os.path.realpath(sys.argv[1]); confirm = sys.argv[2] == "1"; reports = os.path.join(root, ".autospec", "reports")
os.makedirs(reports, exist_ok=True)
protected = {"REPORT_INDEX.md", "report-index.json", "clean-generated-reports.md", "clean-generated-reports.json"}
candidates = [name for name in sorted(os.listdir(reports)) if name not in protected and (name.endswith(".json") or name.endswith(".md"))]
deleted = []
if confirm:
    for name in candidates:
        path = os.path.join(reports, name)
        if os.path.isfile(path):
            os.unlink(path); deleted.append(name)
report = {"schema": 1, "mode": "confirm" if confirm else "dry_run", "candidates": candidates, "deleted": deleted, "scope": "reports_only"}
json.dump(report, open(os.path.join(reports, "clean-generated-reports.json"), "w", encoding="utf-8"), indent=2, sort_keys=True); open(os.path.join(reports, "clean-generated-reports.json"), "a").write("\n")
rows = "\n".join(f"- `{name}`" for name in candidates) or "- None."
open(os.path.join(reports, "clean-generated-reports.md"), "w", encoding="utf-8").write("\n".join(["# Autospec Clean Generated Reports", "", f"Mode: `{'confirm' if confirm else 'dry_run'}`", "", "## Candidate report artifacts", "", rows, "", "State ledgers, work-item history, source specs, and published issue ledgers are not cleaned.", ""]))
print("clean generated reports: " + ("deleted" if confirm else "planned"))
PY

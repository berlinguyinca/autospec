#!/usr/bin/env bash
# scripts/autospec-command-audit.sh — inspect Autospec command consistency.

set -eu

usage() { cat <<'EOF'
Usage: autospec-command-audit.sh [--repo-root DIR]
EOF
}
die() { printf 'autospec-command-audit: %s\n' "$*" >&2; exit 2; }
REPO_ROOT="$(pwd)"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done
[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$(cd "$(dirname "$0")" && pwd -P)" <<'PY'
import json
import os
import re
import sys

root, script_dir = os.path.realpath(sys.argv[1]), os.path.realpath(sys.argv[2])
reports = os.path.join(root, ".autospec", "reports")
runbook = os.path.join(os.path.dirname(script_dir), "docs", "runbooks", "COMMANDS.md")

def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True); fh.write("\n")

def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text.rstrip() + "\n")

runbook_text = open(runbook, encoding="utf-8").read() if os.path.exists(runbook) else ""
commands = []
for path in sorted(p for p in os.listdir(script_dir) if p.startswith("autospec-") and p.endswith(".sh")):
    full = os.path.join(script_dir, path)
    text = open(full, encoding="utf-8", errors="ignore").read()
    writes_github = bool(re.search(r"\bgh\s+|run_gh|GitHub", text))
    local_writes = ".autospec/reports" in text or "write_json" in text or "write_text" in text
    commands.append({
        "command": f"scripts/{path}",
        "exists": True,
        "has_help": "--help" in text or "usage()" in text,
        "supports_dry_run": "--dry-run" in text or "dry-run" in text.lower(),
        "supports_confirm": "--confirm" in text,
        "writes_local_files": local_writes,
        "writes_github": writes_github,
        "requires_network": writes_github,
        "primary_outputs": [".autospec/reports"],
        "runbook_coverage": path in runbook_text,
        "test_coverage_inferable": path.replace(".sh", "") in "\n".join(os.listdir(os.path.join(os.path.dirname(script_dir), "tests", "unit"))) if os.path.isdir(os.path.join(os.path.dirname(script_dir), "tests", "unit")) else False,
    })
findings = [c for c in commands if not c["has_help"]]
report = {"schema": 1, "summary": {"commands_total": len(commands), "missing_help": len(findings)}, "commands": commands, "findings": findings}
write_json(os.path.join(reports, "command-audit.json"), report)
rows = "\n".join(f"| `{c['command']}` | {c['has_help']} | {c['supports_dry_run']} | {c['supports_confirm']} | {c['writes_local_files']} | {c['writes_github']} | {c['runbook_coverage']} |" for c in commands)
write_text(os.path.join(reports, "command-audit.md"), "\n".join([
    "# Autospec Command Audit",
    "",
    "## Summary",
    "",
    f"- Commands total: {len(commands)}",
    f"- Missing help: {len(findings)}",
    "",
    "## Commands",
    "",
    "| Command | Help | Dry-run | Confirm | Local writes | GitHub writes | Runbook |",
    "| --- | --- | --- | --- | --- | --- | --- |",
    rows,
]))
print("command audit: wrote reports")
PY

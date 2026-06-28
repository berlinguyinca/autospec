#!/usr/bin/env bash
# scripts/autospec-audit-to-backlog.sh — connect constitution audit outputs to v3 backlog publishing.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-audit-to-backlog.sh [--repo-root DIR] [--dry-run|--confirm] [--repo OWNER/REPO]

Dry-run is default. Confirm mode may publish issue-plan-v3 issues through
scripts/autospec-publish-issues.sh --confirm --plan v3.
EOF
}

die() {
    printf 'autospec-audit-to-backlog: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
CONFIRM=0
GH_REPO=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        --repo) [ "$#" -ge 2 ] || die "--repo requires OWNER/REPO"; GH_REPO="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$SCRIPT_DIR" "$CONFIRM" "$GH_REPO" <<'PY'
import json
import os
import subprocess
import sys
from datetime import datetime, timezone

root, script_dir, confirm, gh_repo = os.path.realpath(sys.argv[1]), os.path.realpath(sys.argv[2]), sys.argv[3] == "1", sys.argv[4]
reports = os.path.join(root, ".autospec", "reports")
state = os.path.join(root, ".autospec", "state")


def load(path, default):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            data = json.load(fh)
        return data if isinstance(data, dict) else default
    except Exception:
        return default


def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
        fh.write("\n")


def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text.rstrip() + "\n")


def run_optional(command):
    cp = subprocess.run(command, cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return {
        "command": " ".join(command),
        "exit_code": cp.returncode,
        "stdout": cp.stdout.strip(),
        "stderr": cp.stderr.strip(),
        "required": False,
    }


def publish_command(mode):
    command = ["bash", os.path.join(script_dir, "autospec-publish-issues.sh"), "--repo-root", root, mode, "--plan", "v3"]
    if gh_repo:
        command.extend(["--repo", gh_repo])
    return command


issue_plan = load(os.path.join(reports, "issue-plan-v3.json"), {"issues": []})
rule_checks = load(os.path.join(state, "rule-check-results.json"), load(os.path.join(reports, "rule-check-results.json"), {"results": []}))
maturity = load(os.path.join(state, "maturity-score.json"), load(os.path.join(reports, "maturity-score.json"), {"levels": []}))
required_failures = [item for item in rule_checks.get("results", []) if item.get("severity") == "required" and item.get("status") == "fail"]
v3_issues = issue_plan.get("issues", []) if isinstance(issue_plan.get("issues"), list) else []

planned_steps = [
    "validate policy sources",
    "lock policy sources",
    "build Digital Twin",
    "constitution audit v2",
    "issue plan v3",
    "issue publish plan v3",
    "status summary",
]
plan = {
    "version": 1,
    "mode": "confirm" if confirm else "dry_run",
    "planned_steps": planned_steps,
    "issue_plan_v3_present": bool(v3_issues),
    "v3_issue_drafts": len(v3_issues),
    "required_rule_failures": len(required_failures),
    "maturity": maturity,
    "github_writes": bool(confirm),
    "side_effects": {"worker_execution": False, "prs_created": False, "merged": False, "approved": False},
    "next_recommended_command": "bash scripts/autospec-publish-issues.sh --confirm --plan v3" if not confirm else "bash scripts/autospec-autonomy-status.sh",
}
write_json(os.path.join(reports, "audit-to-backlog-plan.json"), plan)
write_text(os.path.join(reports, "audit-to-backlog-plan.md"), "\n".join([
    "# Autospec Audit to Backlog Plan",
    "",
    f"Mode: `{'confirm' if confirm else 'dry_run'}`",
    "",
    "## Summary",
    "",
    f"- V3 issue drafts: {len(v3_issues)}",
    f"- Required rule failures: {len(required_failures)}",
    f"- GitHub writes: `{str(bool(confirm)).lower()}`",
    "",
    "## Planned Chain",
    "",
    "\n".join(f"- {step}" for step in planned_steps),
    "",
    "## Next Recommended Command",
    "",
    f"`{plan['next_recommended_command']}`",
]))

commands = []
status = "planned"
if os.path.isfile(os.path.join(reports, "issue-plan-v3.json")):
    commands.append(run_optional(publish_command("--confirm" if confirm else "--dry-run")))
    status = "published" if confirm and commands[-1]["exit_code"] == 0 else "planned"
else:
    status = "missing_issue_plan_v3"

result = {
    "version": 1,
    "mode": "confirm" if confirm else "dry_run",
    "status": status if all(item["exit_code"] == 0 for item in commands) else "failed",
    "commands": commands,
    "v3_issue_drafts": len(v3_issues),
    "required_rule_failures": len(required_failures),
    "completed_at": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
    "side_effects": {
        "github_writes": bool(confirm),
        "worker_execution": False,
        "prs_created": False,
        "merged": False,
        "approved": False,
    },
}
write_json(os.path.join(reports, "audit-to-backlog-result.json"), result)
write_text(os.path.join(reports, "audit-to-backlog-result.md"), "\n".join([
    "# Autospec Audit to Backlog Result",
    "",
    f"Status: **{result['status']}**",
    "",
    "## Commands",
    "",
    "\n".join(f"- `{item['command']}` -> {item['exit_code']}" for item in commands) or "- None.",
    "",
    "## Safety",
    "",
    "- No worker execution was performed.",
    "- No PR was created.",
    "- No merge or approval was performed.",
]))

print(f"audit to backlog: {result['status']}")
sys.exit(0 if result["status"] in {"planned", "published"} else 1)
PY

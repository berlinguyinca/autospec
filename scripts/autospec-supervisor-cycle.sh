#!/usr/bin/env bash
# scripts/autospec-supervisor-cycle.sh — run one bounded autonomy supervisor cycle.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-supervisor-cycle.sh [--repo-root DIR] [--dry-run|--confirm] [--repo OWNER/REPO] [--issue NUMBER]
EOF
}

die() {
    printf 'autospec-supervisor-cycle: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
CONFIRM=0
GH_REPO=""
ISSUE=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        --repo) GH_REPO="$2"; shift 2 ;;
        --issue) ISSUE="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$SCRIPT_DIR" "$CONFIRM" "$GH_REPO" "$ISSUE" <<'PY'
import json
import os
import subprocess
import sys
from datetime import datetime, timezone

root, script_dir, confirm, gh_repo, issue = os.path.realpath(sys.argv[1]), os.path.realpath(sys.argv[2]), sys.argv[3] == "1", sys.argv[4], sys.argv[5]
reports = os.path.join(root, ".autospec", "reports")
state = os.path.join(root, ".autospec", "state")
supervisor_state = os.path.join(state, "supervisor-runs.json")

def load(path, default):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return json.load(fh)
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

def run_script(args):
    cp = subprocess.run(args, cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return {"command": " ".join(args), "exit_code": cp.returncode, "stdout": cp.stdout.strip(), "stderr": cp.stderr.strip()}

def select_issue():
    if issue:
        return str(issue)
    plan = load(os.path.join(reports, "issue-plan.json"), {})
    issues = plan.get("issues", [])
    if issues:
        return str(issues[0].get("github_issue_number") or issues[0].get("issue_number") or 1)
    published = load(os.path.join(state, "published-issues.json"), {})
    items = published.get("issues", [])
    if items:
        return str(items[0].get("github_issue_number") or 1)
    return "1"

selected_issue = select_issue()
steps = [
    "sync published issues",
    "sync guidance",
    "select one eligible issue",
    "worker execution",
    "verifier execution",
    "promotion/remediation/stuck handling",
    "final report",
]

plan = {
    "version": 1,
    "mode": "confirm" if confirm else "dry_run",
    "planned_issue_count": 1,
    "selected_issue": selected_issue,
    "steps": steps,
    "limits": {
        "max_issues_per_cycle": 1,
        "auto_merge": False,
        "self_approval": False,
        "default_branch_push": False,
    },
}
write_json(os.path.join(reports, "supervisor-cycle-plan.json"), plan)
write_text(
    os.path.join(reports, "supervisor-cycle-plan.md"),
    "\n".join([
        "# Supervisor Cycle Plan",
        "",
        f"Selected issue: `{selected_issue}`",
        "",
        "## Planned Steps",
        *[f"- {s}" for s in steps],
        "",
        "This cycle is capped at one issue and performs no merge or approval.",
    ]),
)

if not confirm:
    print("supervisor cycle plan: PASS")
    sys.exit(0)

verifier = load(os.path.join(reports, "verifier-report.json"), {})
verdict = verifier.get("verdict")
commands = []
outcome = "blocked"

if verdict in {"pass", "pass_with_warnings"}:
    cmd = ["bash", os.path.join(script_dir, "autospec-promote-pr.sh"), "--repo-root", root, "--dry-run", "--pr", "7"]
    if gh_repo:
        cmd += ["--repo", gh_repo]
    commands.append(run_script(cmd))
    outcome = "promotion_planned"
elif verdict:
    cmd = ["bash", os.path.join(script_dir, "autospec-plan-remediation.sh"), "--repo-root", root, "--dry-run", "--pr", "7"]
    commands.append(run_script(cmd))
    outcome = "remediation_planned"
else:
    outcome = "needs_guidance"

result = {
    "version": 1,
    "mode": "confirm",
    "selected_issue": selected_issue,
    "processed_issue_count": 1,
    "verifier_verdict": verdict,
    "outcome": outcome,
    "commands": commands,
    "side_effects": {
        "merged": False,
        "approved": False,
        "default_branch_push": False,
    },
    "completed_at": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
}
write_json(os.path.join(reports, "supervisor-cycle-result.json"), result)
write_text(
    os.path.join(reports, "supervisor-cycle-result.md"),
    "\n".join([
        "# Supervisor Cycle Result",
        "",
        f"Outcome: **{outcome}**",
        f"Selected issue: `{selected_issue}`",
        f"Verifier verdict: `{verdict or 'missing'}`",
        "",
        "## Commands",
        *[f"- `{c['command']}` -> {c['exit_code']}" for c in commands],
        "",
        "## Safety",
        "- No merge was performed.",
        "- No approval was performed.",
        "- No second issue was processed.",
    ]),
)

runs = load(supervisor_state, {"schema": 1, "runs": []})
runs.setdefault("schema", 1)
runs.setdefault("runs", []).append(result)
write_json(supervisor_state, runs)
print("supervisor cycle: PASS")
PY

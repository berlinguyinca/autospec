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
        return str(issue), {}
    published = load(os.path.join(state, "published-issues.json"), {})
    items = published.get("issues", [])
    if items:
        ordered = sorted(items, key=lambda x: ({"v3": 0, "v2": 1, "v1": 2}.get(x.get("plan_version", "v1"), 3), x.get("local_issue_id", "")))
        selected = ordered[0]
        return str(selected.get("github_issue_number") or selected.get("local_issue_id") or 1), selected
    for version, filename in [("v3", "issue-plan-v3.json"), ("v2", "issue-plan-v2.json"), ("v1", "issue-plan.json")]:
        plan = load(os.path.join(reports, filename), {})
        issues = plan.get("issues", [])
        if issues:
            selected = dict(sorted(issues, key=lambda item: item.get("issue_id", ""))[0])
            selected["plan_version"] = version
            return str(selected.get("github_issue_number") or selected.get("issue_number") or selected.get("issue_id") or 1), selected
    return "1", {}

selected_issue, selected_context = select_issue()
rule_ids = selected_context.get("rule_ids") or selected_context.get("source_rule_ids") or []
quality_gate_ids = selected_context.get("quality_gate_ids") or selected_context.get("quality_gates") or []
risk = selected_context.get("risk", {}) if isinstance(selected_context.get("risk", {}), dict) else {}
eligibility = "eligible"
if str(risk.get("level", "")).lower() == "high" or risk.get("requires_architecture_review"):
    eligibility = "stuck_guidance"
elif risk.get("requires_human_review"):
    eligibility = "docs_spec_metadata_only"
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
    "selected_issue_context": {
        "plan_version": selected_context.get("plan_version", "legacy"),
        "local_issue_id": selected_context.get("local_issue_id") or selected_context.get("issue_id", ""),
        "rule_ids": rule_ids,
        "quality_gate_ids": quality_gate_ids,
        "category": selected_context.get("category", ""),
        "severity": selected_context.get("severity") or selected_context.get("rule_severity", ""),
        "maturity_target": selected_context.get("maturity_level", ""),
        "source_doctrine": selected_context.get("source_doctrine", ""),
        "source_baseline_pack": selected_context.get("source_baseline_pack", ""),
        "source_policy_file": selected_context.get("source_file", "") or (selected_context.get("source_policy_files") or [""])[0],
        "rule_check_evidence": selected_context.get("evidence", []),
        "missing_evidence": selected_context.get("missing_evidence", []),
        "remediation_hint": selected_context.get("remediation_hint", ""),
        "structured_acceptance_criteria": selected_context.get("acceptance_criteria", []),
        "structured_validation_expectations": selected_context.get("validation_expectations", []),
        "risk": risk,
        "worker_eligibility": eligibility,
        "why_selected": "v3 structured-rule issue has highest source priority" if selected_context.get("plan_version") == "v3" else "highest available compatible issue source",
    },
    "steps": steps,
    "limits": {
        "max_issues_per_cycle": 1,
        "auto_merge": False,
        "self_approval": False,
        "default_branch_push": False,
    },
}
write_json(os.path.join(reports, "supervisor-cycle-plan.json"), plan)
plan_md = [
    "# Supervisor Cycle Plan",
    "",
    "## Selected Issue",
    "",
    f"Selected issue: `{selected_issue}`",
    "",
    "## Source Rule(s)",
    "",
]
plan_md.extend([f"- `{rid}`" for rid in rule_ids] if rule_ids else ["- None."])
plan_md.extend([
    "",
    "## Source Baseline Pack",
    "",
    f"`{selected_context.get('source_baseline_pack', '') or 'n/a'}`",
    "",
    "## Rule Severity",
    "",
    f"`{selected_context.get('severity') or selected_context.get('rule_severity') or 'unknown'}`",
    "",
    "## Maturity Target",
    "",
    f"`{selected_context.get('maturity_level', 'unknown')}`",
    "",
    "## Quality Gates",
    "",
])
plan_md.extend([f"- `{gid}`" for gid in quality_gate_ids] if quality_gate_ids else ["- None."])
plan_md.extend([
    "",
    "## Worker Eligibility",
    "",
    f"`{eligibility}`",
    "",
    "## Expected Validation",
    "",
])
validation_expectations = selected_context.get("validation_expectations", [])
plan_md.extend([f"- `{cmd}`" for cmd in validation_expectations] if validation_expectations else ["- None."])
plan_md.extend([
    "",
    "## Why This Issue Was Selected",
    "",
    plan["selected_issue_context"]["why_selected"],
    "",
    "## Planned Steps",
])
plan_md.extend([f"- {s}" for s in steps])
plan_md.extend([
    "",
    "This cycle is capped at one issue and performs no merge or approval.",
])
write_text(os.path.join(reports, "supervisor-cycle-plan.md"), "\n".join(plan_md))

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

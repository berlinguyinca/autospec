#!/usr/bin/env bash
# scripts/autospec-autonomy-status.sh — summarize local autospec autonomy state.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-autonomy-status.sh [--repo-root DIR]
EOF
}

die() {
    printf 'autospec-autonomy-status: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) REPO_ROOT="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" <<'PY'
import glob
import json
import os
import sys

root = os.path.realpath(sys.argv[1])
reports = os.path.join(root, ".autospec", "reports")
state = os.path.join(root, ".autospec", "state")
run_lock = os.path.join(root, ".autospec", "run.lock")
stop_flag = os.path.expanduser("~/.autospec/stop.flag")

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

published = load(os.path.join(state, "published-issues.json"), {"issues": []})
handover = load(os.path.join(state, "stuck-handovers.json"), {"handovers": []})
supervisor = load(os.path.join(state, "supervisor-runs.json"), {"runs": []})
issue_plan = load(os.path.join(reports, "issue-plan.json"), {"issues": []})
budget = load(os.path.join(reports, "autonomy-budget.json"), {"overall_status": "unknown", "budgets": []})
repeated = load(os.path.join(reports, "repeated-failures.json"), {"has_repeated_failures": False, "repeated_failures": []})
rule_checks = load(os.path.join(reports, "rule-check-results.json"), {"results": []})
maturity = load(os.path.join(reports, "maturity-score.json"), {"levels": []})
audit = load(os.path.join(reports, "constitution-audit.json"), {})
policy_sources = load(os.path.join(state, "policy-sources.json"), {})
policy_validation = load(os.path.join(reports, "policy-source-validation.json"), {})
policy_compatibility = load(os.path.join(reports, "policy-compatibility.json"), {})

promotions = [load(path, {}) for path in sorted(glob.glob(os.path.join(state, "promotions", "*.json")))]
verifications = [load(path, {}) for path in sorted(glob.glob(os.path.join(state, "verifications", "*.json")))]

managed = len(published.get("issues", []))
verified = sum(1 for item in promotions if "autospec:verified" in item.get("labels_to_add", []) or item.get("promotion_allowed"))
needs_review = sum(1 for item in promotions if "autospec:needs-human-review" in item.get("labels_to_add", []))
stuck = sum(1 for item in handover.get("handovers", []) if item.get("state") in {"needs-guidance", "stuck"})
ready = sum(1 for item in handover.get("handovers", []) if item.get("state") == "ready-to-resume")
guidance = sum(1 for item in handover.get("handovers", []) if item.get("guidance_detected") or item.get("state") == "ready-to-resume")
blocked = sum(1 for item in verifications if item.get("verdict") in {"blocked", "needs_guidance"})
locked = os.path.isdir(run_lock)
stopped = os.path.exists(stop_flag)
budget_exhausted = budget.get("overall_status") == "exhausted"
needs_guidance = stuck > 0 or blocked > 0 or bool(repeated.get("has_repeated_failures"))
ready_to_run = not locked and not stopped and not budget_exhausted and not needs_guidance
required_rule_failures = [r for r in rule_checks.get("results", []) if r.get("severity") == "required" and r.get("status") == "fail"]
production_maturity = next((level for level in maturity.get("levels", []) if level.get("level") == "production"), {})
issue_plan_v2 = os.path.join(reports, "issue-plan-v2.json")
issue_plan_v3 = os.path.join(reports, "issue-plan-v3.json")
issue_plan_v1 = os.path.join(reports, "issue-plan.json")
issue_plan_v2_newer = os.path.exists(issue_plan_v2) and (not os.path.exists(issue_plan_v1) or os.path.getmtime(issue_plan_v2) >= os.path.getmtime(issue_plan_v1))
issue_plan_v3_newer = os.path.exists(issue_plan_v3) and (not os.path.exists(issue_plan_v1) or os.path.getmtime(issue_plan_v3) >= os.path.getmtime(issue_plan_v1))
lockfile = os.path.join(root, ".autospec", "policy-sources.lock.json")
extraction = load(os.path.join(reports, "rule-extraction.json"), {})

summary = {
    "managed_issues": managed,
    "open_worker_prs": len(verifications),
    "verified_prs": verified,
    "needs_human_review": needs_review,
    "stuck_items": stuck,
    "blocked_items": blocked,
    "guidance_provided": guidance,
    "ready_to_resume": ready,
    "ready_to_run": ready_to_run,
    "stopped": stopped,
    "locked": locked,
    "budget_exhausted": budget_exhausted,
    "needs_guidance": needs_guidance,
}
report = {
    "version": 1,
    "summary": summary,
    "current_active_work": published.get("issues", []),
    "stuck_guidance_queue": handover.get("handovers", []),
    "pr_review_queue": promotions,
    "recent_supervisor_runs": supervisor.get("runs", [])[-5:],
    "loop_readiness": {
        "ready_to_run": ready_to_run,
        "lock_status": "locked" if locked else "unlocked",
        "stop_flag_status": "present" if stopped else "absent",
        "budget_status": budget.get("overall_status", "unknown"),
        "repeated_failures": repeated.get("has_repeated_failures", False),
    },
    "budget": budget,
    "repeated_failures": repeated,
    "rule_audit": {
        "fresh": bool(rule_checks.get("results")),
        "constitution_audit_status": audit.get("status", "unknown"),
        "policy_source_status": policy_validation.get("status", "unknown"),
        "policy_lockfile_present": os.path.exists(lockfile),
        "structured_rules": extraction.get("structured_rules", 0),
        "heuristic_rules": extraction.get("heuristic_rules", 0),
        "policy_compatibility_status": policy_compatibility.get("status", "unknown"),
        "policy_compatibility_warnings": len(policy_compatibility.get("findings", [])),
        "maturity_status": production_maturity.get("status", "unknown"),
        "maturity_score": production_maturity.get("score", 0.0),
        "top_failing_required_rules": [r.get("rule_id") for r in required_rule_failures[:5]],
        "expired_waivers": [f for f in load(os.path.join(reports, "rule-extraction.json"), {}).get("waiver_findings", []) if f.get("code") == "WAIVER_EXPIRED"],
        "issue_plan_v2_newer_than_v1": issue_plan_v2_newer,
        "issue_plan_v3_newer_than_v1": issue_plan_v3_newer,
        "latest_constitution_audit_v2": audit.get("generated_at", "unknown"),
    },
    "planned_backlog_items": len(issue_plan.get("issues", [])),
    "guide_skill_quick_commands": [
        "autospec-guide: What is autospec working on?",
        "autospec-guide: What is stuck?",
        "autospec-guide: What command should I run next?",
    ],
    "top_recommended_next_commands": [
        "bash scripts/autospec-constitution-audit.sh",
        "bash scripts/autospec-validate-policy-sources.sh",
        "bash scripts/autospec-policy-compatibility.sh",
        "bash scripts/autospec-build-digital-twin.sh",
        "bash scripts/autospec-autonomy-status.sh",
        "bash scripts/autospec-supervisor-loop.sh --dry-run --max-cycles 3",
        "bash scripts/autospec-supervisor-cycle.sh --dry-run --issue <number>",
        "bash scripts/autospec-sync-guidance.sh --dry-run",
        "bash scripts/autospec-promote-pr.sh --dry-run --pr <number>",
    ],
}
write_json(os.path.join(reports, "autonomy-status.json"), report)

cards = [
    ("Managed issues", managed),
    ("Open worker PRs", len(verifications)),
    ("Verified PRs", verified),
    ("Needs human review", needs_review),
    ("Stuck items", stuck),
    ("Blocked items", blocked),
    ("Guidance provided", guidance),
    ("Ready to resume", ready),
    ("Ready to run", int(ready_to_run)),
    ("Stopped", int(stopped)),
    ("Locked", int(locked)),
    ("Budget exhausted", int(budget_exhausted)),
    ("Needs guidance", int(needs_guidance)),
]
md = [
    "# Autospec Autonomy Status",
    "",
    "## Summary Cards",
    "",
    "| Metric | Count |",
    "| --- | ---: |",
    *[f"| {name} | {value} |" for name, value in cards],
    "",
    "## Loop Readiness",
    "",
    "| Check | Status |",
    "| --- | --- |",
    f"| Ready to run | {str(ready_to_run).lower()} |",
    f"| Lock status | {'locked' if locked else 'unlocked'} |",
    f"| Stop flag status | {'present' if stopped else 'absent'} |",
    f"| Budget status | {budget.get('overall_status', 'unknown')} |",
    f"| Repeated failures | {str(repeated.get('has_repeated_failures', False)).lower()} |",
    "",
    "## Rule Audit",
    "",
    "| Check | Status |",
    "| --- | --- |",
    f"| Rule check freshness | {str(bool(rule_checks.get('results'))).lower()} |",
    f"| Structured policy sources | constitution={policy_sources.get('constitution', {}).get('structured_available', False)} baselines={policy_sources.get('baselines', {}).get('structured_available', False)} |",
    f"| Policy validation status | {policy_validation.get('status', 'unknown')} |",
    f"| Policy lockfile | {'present' if os.path.exists(lockfile) else 'missing'} |",
    f"| Structured vs heuristic rules | {extraction.get('structured_rules', 0)} / {extraction.get('heuristic_rules', 0)} |",
    f"| Policy compatibility | {policy_compatibility.get('status', 'unknown')} ({len(policy_compatibility.get('findings', []))} findings) |",
    f"| Constitution audit status | {audit.get('status', 'unknown')} |",
    f"| Production maturity | {production_maturity.get('status', 'unknown')} ({production_maturity.get('score', 0.0)}) |",
    f"| Issue plan v2 newer than v1 | {str(issue_plan_v2_newer).lower()} |",
    f"| Issue plan v3 newer than v1 | {str(issue_plan_v3_newer).lower()} |",
    "",
    "### Top Failing Required Rules",
    "",
]
if required_rule_failures:
    for rule in required_rule_failures[:5]:
        md.append(f"- `{rule.get('rule_id')}`")
else:
    md.append("- None.")
md += [
    "",
    "## Current Active Work",
    "",
    "| Issue | State | Labels |",
    "| --- | --- | --- |",
]
if published.get("issues"):
    for item in published.get("issues", []):
        md.append(f"| {item.get('github_issue_number', item.get('local_issue_id', 'unknown'))} | {item.get('state', 'unknown')} | {', '.join(item.get('labels', []))} |")
else:
    md.append("| none | none | none |")
md += [
    "",
    "## Stuck / Guidance Queue",
    "",
    "| Work item | Stuck issue | State |",
    "| --- | --- | --- |",
]
if handover.get("handovers"):
    for item in handover.get("handovers", []):
        md.append(f"| {item.get('work_item_id')} | {item.get('stuck_issue_number', 'n/a')} | {item.get('state', 'unknown')} |")
else:
    md.append("| none | none | none |")
md += [
    "",
    "## PR Review Queue",
    "",
    "| Source | Promotion allowed | Labels |",
    "| --- | --- | --- |",
]
if promotions:
    for item in promotions:
        source = item.get("source", {})
        md.append(f"| PR {source.get('pr', 'unknown')} | {item.get('promotion_allowed', False)} | {', '.join(item.get('labels_to_add', []))} |")
else:
    md.append("| none | false | none |")
md += [
    "",
    "## Recent Supervisor Runs",
    "",
    "| Issue | Outcome | Verdict |",
    "| --- | --- | --- |",
]
if supervisor.get("runs"):
    for item in supervisor.get("runs", [])[-5:]:
        md.append(f"| {item.get('selected_issue', 'unknown')} | {item.get('outcome', 'unknown')} | {item.get('verifier_verdict', 'unknown')} |")
else:
    md.append("| none | none | none |")
md += [
    "",
    "## Budget Status",
    "",
    "| Budget | Used | Limit | Status |",
    "| --- | ---: | ---: | --- |",
]
for row in budget.get("budgets", []):
    md.append(f"| {row.get('budget')} | {row.get('used')} | {row.get('limit')} | {row.get('status')} |")
if not budget.get("budgets"):
    md.append("| unknown | 0 | 0 | unknown |")
md += [
    "",
    "## Repeated Failures",
    "",
    "| Kind | Subject | Count |",
    "| --- | --- | ---: |",
]
for item in repeated.get("repeated_failures", []):
    md.append(f"| {item.get('kind')} | {item.get('subject')} | {item.get('count')} |")
if not repeated.get("repeated_failures"):
    md.append("| none | none | 0 |")
md += [
    "",
    "## Guide Skill Quick Commands",
    "",
    *[f"- `{cmd}`" for cmd in report["guide_skill_quick_commands"]],
    "",
    "## Top Recommended Next Commands",
    "",
    *[f"- `{cmd}`" for cmd in report["top_recommended_next_commands"]],
]
write_text(os.path.join(reports, "autonomy-status.md"), "\n".join(md))
print("autonomy status: PASS")
PY

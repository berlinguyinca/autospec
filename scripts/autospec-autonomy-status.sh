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

promotions = [load(path, {}) for path in sorted(glob.glob(os.path.join(state, "promotions", "*.json")))]
verifications = [load(path, {}) for path in sorted(glob.glob(os.path.join(state, "verifications", "*.json")))]

managed = len(published.get("issues", []))
verified = sum(1 for item in promotions if "autospec:verified" in item.get("labels_to_add", []) or item.get("promotion_allowed"))
needs_review = sum(1 for item in promotions if "autospec:needs-human-review" in item.get("labels_to_add", []))
stuck = sum(1 for item in handover.get("handovers", []) if item.get("state") in {"needs-guidance", "stuck"})
ready = sum(1 for item in handover.get("handovers", []) if item.get("state") == "ready-to-resume")
guidance = sum(1 for item in handover.get("handovers", []) if item.get("guidance_detected") or item.get("state") == "ready-to-resume")
blocked = sum(1 for item in verifications if item.get("verdict") in {"blocked", "needs_guidance"})

summary = {
    "managed_issues": managed,
    "open_worker_prs": len(verifications),
    "verified_prs": verified,
    "needs_human_review": needs_review,
    "stuck_items": stuck,
    "blocked_items": blocked,
    "guidance_provided": guidance,
    "ready_to_resume": ready,
}
report = {
    "version": 1,
    "summary": summary,
    "current_active_work": published.get("issues", []),
    "stuck_guidance_queue": handover.get("handovers", []),
    "pr_review_queue": promotions,
    "recent_supervisor_runs": supervisor.get("runs", [])[-5:],
    "planned_backlog_items": len(issue_plan.get("issues", [])),
    "top_recommended_next_commands": [
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
    "## Top Recommended Next Commands",
    "",
    *[f"- `{cmd}`" for cmd in report["top_recommended_next_commands"]],
]
write_text(os.path.join(reports, "autonomy-status.md"), "\n".join(md))
print("autonomy status: PASS")
PY

#!/usr/bin/env bash
# scripts/autospec-autonomy-dry-run.sh — run local autonomy dry-run reporting.
#
# Local filesystem only. Orchestrates available Constitution/Baseline scripts and
# writes a human-readable dry-run report without GitHub/API/network side effects.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-autonomy-dry-run.sh [--repo-root <dir>]

Writes:
  .autospec/reports/autonomy-dry-run.json
  .autospec/reports/autonomy-dry-run.md
EOF
}

die() {
    printf 'autospec-autonomy-dry-run: %s\n' "$*" >&2
    exit 2
}

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

run_stage() {
    stage_name="$1"
    script_path="$2"
    required="$3"
    shift 3
    if [ ! -x "$script_path" ]; then
        printf 'stage:%s: skipped missing %s\n' "$stage_name" "$script_path"
        [ "$required" = "required" ] && return 2
        return 0
    fi
    set +e
    "$script_path" --repo-root "$REPO_ROOT" "$@" >/tmp/autospec-dry-run-stage.log 2>&1
    code="$?"
    set -e
    # Exit 1 means validation/gaps found for several local reports, not a dry-run failure.
    if [ "$code" -gt 1 ]; then
        cat /tmp/autospec-dry-run-stage.log >&2
        return "$code"
    fi
    printf 'stage:%s: exit %s\n' "$stage_name" "$code"
    return 0
}

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
run_stage "constitution/baseline validation" "$SCRIPT_DIR/autospec-constitution-validate.sh" optional || true
run_stage "baseline composition" "$SCRIPT_DIR/autospec-baseline-compose.sh" optional || true
run_stage "metadata discovery" "$SCRIPT_DIR/autospec-discover-metadata.sh" optional
run_stage "baseline gap analysis" "$SCRIPT_DIR/autospec-baseline-gap.sh" optional || true
run_stage "constitutional gap report" "$SCRIPT_DIR/autospec-constitutional-gap.sh" optional || true
run_stage "issue planning" "$SCRIPT_DIR/autospec-plan-issues.sh" required
run_stage "control-plane report generation" "$SCRIPT_DIR/autospec-bot-state-init.sh" required || run_stage "control-plane report generation" "$SCRIPT_DIR/autospec-bot-state-init.sh" required --force

python3 - "$REPO_ROOT" <<'PY'
import json
import os
import sys

repo_root = os.path.realpath(sys.argv[1])
reports_dir = os.path.join(repo_root, ".autospec", "reports")
state_dir = os.path.join(repo_root, ".autospec", "state")
templates_dir = os.path.join(repo_root, ".autospec", "templates")
json_path = os.path.join(reports_dir, "autonomy-dry-run.json")
md_path = os.path.join(reports_dir, "autonomy-dry-run.md")


def load_json(path, default):
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


metadata = load_json(os.path.join(reports_dir, "metadata-discovery.json"), {})
composition = load_json(os.path.join(reports_dir, "baseline-composition.json"), {})
baseline_gap = load_json(os.path.join(reports_dir, "baseline-gap-analysis.json"), {})
issue_plan = load_json(os.path.join(reports_dir, "issue-plan.json"), {"issues": []})
control_plane_exists = os.path.isfile(os.path.join(state_dir, "bot-control-plane.json"))
control_labels_exists = os.path.isfile(os.path.join(state_dir, "control-labels.yml"))
state_machine_exists = os.path.isfile(os.path.join(state_dir, "bot-state-machine.yml"))
stuck_template_exists = os.path.isfile(os.path.join(templates_dir, "stuck-issue.md"))

facts = metadata.get("facts", {})
languages = facts.get("languages", {}).get("value", [])
package_managers = facts.get("package_managers", {}).get("value", [])
repo_name = facts.get("repo_name", {}).get("value", os.path.basename(repo_root))
profiles = composition.get("baselines", {}).get("requested_profiles") or sorted({
    item.get("profile", "")
    for item in composition.get("composed", {}).get("capabilities", [])
    if item.get("profile")
})
gaps = [
    item for item in baseline_gap.get("matrix", [])
    if item.get("status") not in {"present", "opted_out"}
]
issues = issue_plan.get("issues", []) if isinstance(issue_plan.get("issues"), list) else []
high_risk = [issue for issue in issues if issue.get("risk") == "High" or "autospec:risk-high" in issue.get("suggested_labels", []) or issue.get("planning_bucket", {}).get("rank") == 7]

report = {
    "version": 1,
    "status": "pass",
    "repo": repo_name,
    "app_stack": {
        "languages": languages,
        "package_managers": package_managers,
        "api": metadata.get("indicators", {}).get("api", {}).get("value"),
        "ui": metadata.get("indicators", {}).get("ui", {}).get("value"),
        "ai_rag": metadata.get("indicators", {}).get("ai_rag", {}).get("value"),
    },
    "selected_baseline_profiles": profiles,
    "top_gaps": gaps[:10],
    "proposed_issue_backlog": [
        {
            "issue_id": issue.get("issue_id"),
            "title": issue.get("title"),
            "priority": issue.get("priority"),
            "risk": issue.get("risk"),
            "depends_on": issue.get("depends_on", []),
            "draft_path": issue.get("draft_path"),
        }
        for issue in issues
    ],
    "high_risk_items": [
        {
            "issue_id": issue.get("issue_id"),
            "title": issue.get("title"),
            "risk": issue.get("risk"),
            "blocked_reason": issue.get("blocked_reason"),
        }
        for issue in high_risk
    ],
    "control_plane": {
        "bot_control_plane": control_plane_exists,
        "control_labels": control_labels_exists,
        "bot_state_machine": state_machine_exists,
        "stuck_issue_template": stuck_template_exists,
    },
    "side_effects": {
        "github_api_calls": False,
        "github_issues_created": False,
        "branches_created": False,
        "prs_created": False,
        "implementation_started": False,
        "network_required": False,
    },
    "recommended_next_command": "bash scripts/autospec-build-digital-twin.sh --repo-root <repo>",
}
write_json(json_path, report)

lines = [
    "# Autospec Autonomy Dry Run",
    "",
    "Status: **PASS**",
    "",
    "## Detected App Type / Stack",
    "",
    f"- Repository: `{repo_name}`",
    f"- Languages: {', '.join(languages) if languages else 'unknown'}",
    f"- Package managers: {', '.join(package_managers) if package_managers else 'none detected'}",
    f"- API indicators: {metadata.get('indicators', {}).get('api', {}).get('value')}",
    f"- UI indicators: {metadata.get('indicators', {}).get('ui', {}).get('value')}",
    f"- AI/RAG indicators: {metadata.get('indicators', {}).get('ai_rag', {}).get('value')}",
    "",
    "## Selected Baseline Profiles",
    "",
]
lines.extend([f"- `{profile}`" for profile in profiles] or ["- None detected."])
lines.extend(["", "## Top Gaps", "", "| Family | Capability | Status | Priority | Evidence |", "| --- | --- | --- | --- | --- |"])
if gaps:
    for gap in gaps[:10]:
        lines.append(f"| {gap.get('feature_family', '')} | {gap.get('capability', '')} | {gap.get('status', '')} | {gap.get('priority', '')} | {'; '.join(gap.get('evidence', [])[:2])} |")
else:
    lines.append("| n/a | none | present | none | No open baseline gaps. |")
lines.extend(["", "## Proposed Issue Backlog", "", "| ID | Title | Priority | Risk | Depends on | Draft |", "| --- | --- | --- | --- | --- | --- |"])
if issues:
    for issue in issues:
        depends = ", ".join(issue.get("depends_on", [])) if issue.get("depends_on") else "none"
        lines.append(f"| `{issue.get('issue_id', '')}` | {issue.get('title', '')} | {issue.get('priority', '')} | {issue.get('risk', '')} | {depends} | `{issue.get('draft_path', '')}` |")
else:
    lines.append("| n/a | No issue drafts proposed. | n/a | n/a | none | n/a |")
lines.extend(["", "## High-Risk Items", "", "| ID | Title | Reason |", "| --- | --- | --- |"])
if high_risk:
    for issue in high_risk:
        lines.append(f"| `{issue.get('issue_id', '')}` | {issue.get('title', '')} | {issue.get('blocked_reason') or issue.get('planning_bucket', {}).get('name', '')} |")
else:
    lines.append("| n/a | None detected. | n/a |")
lines.extend([
    "",
    "## Stuck / Guidance Protocol",
    "",
    "- Stuck work uses `.autospec/templates/stuck-issue.md`.",
    "- Resume requires guidance plus `autospec:guidance-provided` or `autospec:resume`.",
    "- State transitions are documented in `.autospec/state/bot-state-machine.yml` and `.autospec/reports/bot-state-machine.md`.",
    "",
    "## Recommended Next Command",
    "",
    "```bash",
    "bash scripts/autospec-build-digital-twin.sh --repo-root <repo>",
    "bash scripts/autospec-autonomy-dry-run.sh --repo-root <repo>",
    "```",
    "",
    "## Safety",
    "",
    "- GitHub API calls: false",
    "- GitHub issues created: false",
    "- Branches created: false",
    "- PRs created: false",
    "- Implementation started: false",
])
with open(md_path, "w", encoding="utf-8") as fh:
    fh.write("\n".join(lines))
    fh.write("\n")

print("autonomy dry-run: PASS")
print("reports: .autospec/reports/autonomy-dry-run.json, .autospec/reports/autonomy-dry-run.md")
PY

#!/usr/bin/env bash
# scripts/autospec-plan-issues.sh — turn discovered gaps into dry-run issue drafts.
#
# Local filesystem only. Reads prior Constitution/Baseline intelligence reports
# and writes a deterministic backlog under .autospec/backlog plus summary
# reports. Does not call GitHub, create branches, or start implementation.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-plan-issues.sh [--repo-root <dir>]

Inputs:
  .autospec/reports/baseline-gap-analysis.json
  .autospec/reports/constitutional-gap-report.json
  .autospec/reports/metadata-discovery.json
  .autospec/reports/baseline-composition.json

Writes:
  .autospec/reports/issue-plan.json
  .autospec/reports/issue-plan.md
  .autospec/backlog/issues/*.md
EOF
}

die() {
    printf 'autospec-plan-issues: %s\n' "$*" >&2
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

python3 - "$REPO_ROOT" <<'PY'
import json
import os
import re
import sys

repo_root = os.path.realpath(sys.argv[1])
reports_dir = os.path.join(repo_root, ".autospec", "reports")
backlog_dir = os.path.join(repo_root, ".autospec", "backlog", "issues")
issue_plan_json = os.path.join(reports_dir, "issue-plan.json")
issue_plan_md = os.path.join(reports_dir, "issue-plan.md")


def load_json(path):
    with open(path, "r", encoding="utf-8") as fh:
        data = json.load(fh)
    return data if isinstance(data, dict) else {}


def optional_json(path):
    try:
        return load_json(path)
    except Exception:
        return {}


def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
        fh.write("\n")


def slugify(title):
    text = title.lower().replace(":", "")
    text = re.sub(r"[^a-z0-9]+", "-", text).strip("-")
    return text or "issue"


def md_list(items):
    return "\n".join(f"- {item}" for item in items) if items else "- None."


def ac_list(items):
    return "\n".join(f"- [ ] {item.rstrip('.') }." for item in items) if items else "- [ ] Gap is resolved and validation evidence is attached."


def category_for(capability, section=""):
    text = f"{capability} {section}".lower()
    if "doc" in text:
        return "documentation"
    if "test" in text or "qa" in text or "playwright" in text or "e2e" in text:
        return "testing"
    if "ui" in text or "ux" in text or "frontend" in text or "web" in text:
        return "web"
    if "api" in text or "route" in text or "http" in text:
        return "api"
    if "ai" in text or "rag" in text or "llm" in text:
        return "ai-platform"
    if "operation" in text or "ci" in text or "container" in text:
        return "operations"
    return "baseline"


def risk_for(status, priority):
    if priority == "high" or status == "missing":
        return "Medium"
    if status == "unknown":
        return "Low"
    return "Low"


def docs_required(category):
    if category in {"documentation", "web", "api"}:
        return ["README.md", "docs/USER_MANUAL.md"]
    if category == "testing":
        return ["docs/TESTING.md or equivalent testing section"]
    return ["README.md or relevant docs/ runbook"]


def tests_required(category):
    if category == "web":
        return ["Playwright or e2e coverage for user-visible flow"]
    if category == "api":
        return ["API smoke or integration coverage"]
    if category == "testing":
        return ["Representative unit or smoke tests proving the baseline evidence"]
    return ["Focused regression or validation coverage for the changed capability"]


def validation_for(category):
    if category == "web":
        return "bash scripts/validate.sh && bats tests/unit tests/smoke"
    return "bash scripts/validate.sh"


def issue_from_gap(number, gap, constitutional=None):
    capability = str(gap.get("capability", "baseline"))
    profile = str(gap.get("feature_family", "baseline"))
    status = str(gap.get("status", "unknown"))
    priority = str(gap.get("priority", "medium")).capitalize()
    if priority == "None":
        priority = "Medium"
    title = str(gap.get("suggested_issue_title") or f"feat: close {capability} baseline gap")
    section_name = constitutional.get("section", "") if constitutional else ""
    category = category_for(capability, section_name)
    evidence = [str(item) for item in gap.get("evidence", [])] or ["gap analysis did not provide evidence"]
    confidence = float(gap.get("confidence", 0.0) or 0.0)
    acceptance = []
    if constitutional:
        acceptance.extend(str(item) for item in constitutional.get("acceptance_criteria", []))
    if not acceptance:
        acceptance = [
            f"{capability} evidence is discoverable by Autospec metadata discovery.",
            f"Baseline gap analysis reports `{capability}` as present or opted_out.",
        ]
    suggested_labels = ["autospec:managed", "autospec:discovered", f"autospec:{category}"]
    if profile and profile != "baseline":
        suggested_labels.append(f"baseline:{profile}")
    return {
        "number": number,
        "title": title,
        "summary": f"Close the `{capability}` gap discovered for the `{profile}` baseline profile.",
        "source_gap": {
            "type": "baseline",
            "feature_family": profile,
            "capability": capability,
            "status": status,
            "constitutional_section": section_name,
        },
        "source_reference": {
            "baseline": f"{profile} baseline",
            "doctrine": section_name.replace("_", " ").title() if section_name else "Autospec Constitution",
        },
        "evidence": evidence,
        "confidence": confidence,
        "priority": priority,
        "risk": risk_for(status, priority.lower()),
        "suggested_labels": suggested_labels,
        "dependencies": [],
        "implementation_scope": [
            f"Add or expose repository evidence for `{capability}`.",
            "Keep implementation limited to files needed to satisfy the gap.",
        ],
        "non_goals": [
            "Do not create GitHub issues from this draft.",
            "Do not start autonomous implementation from this draft.",
            "Do not change unrelated application behavior.",
        ],
        "acceptance_criteria": acceptance,
        "suggested_validation_command": validation_for(category),
        "required_docs_updates": docs_required(category),
        "required_tests": tests_required(category),
        "metadata_files_expected_to_change": [
            ".autospec/reports/metadata-discovery.json",
            ".autospec/reports/baseline-gap-analysis.json",
            ".autospec/reports/constitutional-gap-report.json",
        ],
    }


def constitutional_issue_lookup(report):
    lookup = {}
    for section, data in sorted((report.get("sections") or {}).items()):
        for item in data.get("suggested_issues", []) or []:
            title = str(item.get("title", ""))
            if title:
                lookup[title] = {
                    "section": section,
                    "acceptance_criteria": [str(ac) for ac in item.get("acceptance_criteria", [])],
                }
    for item in report.get("next_recommended_issues", []) or []:
        title = str(item.get("title", ""))
        if title and title not in lookup:
            lookup[title] = {
                "section": "next_recommended_issues",
                "acceptance_criteria": [str(ac) for ac in item.get("acceptance_criteria", [])],
            }
    return lookup


def write_issue_file(issue):
    filename = f"{issue['number']:03d}-{slugify(issue['title'])}.md"
    path = os.path.join(backlog_dir, filename)
    lines = [
        f"# {issue['title']}",
        "",
        "## Summary",
        "",
        issue["summary"],
        "",
        "## Source",
        "",
        f"Baseline: {issue['source_reference']['baseline']}",
        f"Doctrine: {issue['source_reference']['doctrine']}",
        "",
        "## Gap",
        "",
        f"Capability `{issue['source_gap']['capability']}` is `{issue['source_gap']['status']}` for `{issue['source_gap']['feature_family']}`.",
        "",
        "## Evidence",
        "",
        md_list(issue["evidence"]),
        "",
        "## Confidence",
        "",
        f"{issue['confidence']:.2f}",
        "",
        "## Priority",
        "",
        issue["priority"],
        "",
        "## Risk",
        "",
        issue["risk"],
        "",
        "## Suggested labels",
        "",
        md_list(issue["suggested_labels"]),
        "",
        "## Dependencies",
        "",
        md_list(issue["dependencies"]),
        "",
        "## Implementation scope",
        "",
        md_list(issue["implementation_scope"]),
        "",
        "## Non-goals",
        "",
        md_list(issue["non_goals"]),
        "",
        "## Acceptance criteria",
        "",
        ac_list(issue["acceptance_criteria"]),
        "",
        "## Validation",
        "",
        "Run:",
        "",
        "```bash",
        issue["suggested_validation_command"],
        "```",
        "",
        "## Required docs updates",
        "",
        md_list(issue["required_docs_updates"]),
        "",
        "## Required tests",
        "",
        md_list(issue["required_tests"]),
        "",
        "## Metadata files expected to change",
        "",
        md_list(issue["metadata_files_expected_to_change"]),
        "",
    ]
    with open(path, "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines))
    issue["draft_path"] = os.path.relpath(path, repo_root).replace(os.sep, "/")


required = [
    "baseline-gap-analysis.json",
    "constitutional-gap-report.json",
    "metadata-discovery.json",
    "baseline-composition.json",
]
missing = [name for name in required if not os.path.isfile(os.path.join(reports_dir, name))]
if missing:
    os.makedirs(reports_dir, exist_ok=True)
    report = {
        "version": 1,
        "status": "error",
        "errors": [{"code": "INPUT_MISSING", "message": f"missing report: {name}"} for name in missing],
        "issues": [],
    }
    write_json(issue_plan_json, report)
    print("issue planning: ERROR")
    for name in missing:
        print(f"- missing report: {name}")
    sys.exit(2)

baseline_gap = load_json(os.path.join(reports_dir, "baseline-gap-analysis.json"))
constitutional_gap = load_json(os.path.join(reports_dir, "constitutional-gap-report.json"))
metadata = optional_json(os.path.join(reports_dir, "metadata-discovery.json"))
composition = optional_json(os.path.join(reports_dir, "baseline-composition.json"))
constitution_lookup = constitutional_issue_lookup(constitutional_gap)
candidate_gaps = [
    (idx, gap) for idx, gap in enumerate(baseline_gap.get("matrix", []))
    if gap.get("status") not in {"present", "opted_out"} and gap.get("suggested_issue_title")
]
candidate_gaps.sort(key=lambda item: (
    {"high": 0, "medium": 1, "low": 2, "none": 3}.get(str(item[1].get("priority", "")).lower(), 2),
    item[0],
))

os.makedirs(backlog_dir, exist_ok=True)
for name in os.listdir(backlog_dir):
    if name.endswith(".md"):
        os.unlink(os.path.join(backlog_dir, name))

issues = []
for idx, (_, gap) in enumerate(candidate_gaps, start=1):
    title = str(gap.get("suggested_issue_title"))
    issue = issue_from_gap(idx, gap, constitution_lookup.get(title))
    write_issue_file(issue)
    issues.append(issue)

plan = {
    "version": 1,
    "status": "pass",
    "mode": "dry_run",
    "inputs": {
        "baseline_gap_analysis": ".autospec/reports/baseline-gap-analysis.json",
        "constitutional_gap_report": ".autospec/reports/constitutional-gap-report.json",
        "metadata_discovery": ".autospec/reports/metadata-discovery.json",
        "baseline_composition": ".autospec/reports/baseline-composition.json",
    },
    "repo": metadata.get("facts", {}).get("repo_name", {}).get("value", ""),
    "baseline_profiles": composition.get("baselines", {}).get("requested_profiles", []),
    "issue_count": len(issues),
    "issues": issues,
    "side_effects": {
        "github_writes": False,
        "branches_created": False,
        "prs_created": False,
        "implementation_started": False,
    },
}
write_json(issue_plan_json, plan)

lines = [
    "# Issue Plan",
    "",
    "Status: **PASS**",
    "",
    f"- Mode: `{plan['mode']}`",
    f"- Draft issues: {len(issues)}",
    "",
    "| # | Title | Priority | Risk | Draft |",
    "| ---: | --- | --- | --- | --- |",
]
for issue in issues:
    lines.append(f"| {issue['number']} | {issue['title']} | {issue['priority']} | {issue['risk']} | `{issue['draft_path']}` |")
lines.extend(["", "## Safety", "", "- GitHub writes: false", "- Branches created: false", "- PRs created: false", "- Autonomous implementation started: false", ""])
with open(issue_plan_md, "w", encoding="utf-8") as fh:
    fh.write("\n".join(lines))

print("issue planning: PASS")
print(f"draft issues: {len(issues)}")
print("reports: .autospec/reports/issue-plan.json, .autospec/reports/issue-plan.md")
PY

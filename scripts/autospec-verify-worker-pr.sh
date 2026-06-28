#!/usr/bin/env bash
# scripts/autospec-verify-worker-pr.sh — independent verifier for worker output.
#
# Dry-run by default. The verifier writes reports/state, may comment on a PR
# only with --confirm, and never approves, merges, pushes, or fixes code.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-verify-worker-pr.sh [--repo-root <dir>] [--dry-run|--confirm] [--repo OWNER/REPO] (--issue <number>|--pr <number>|--work-item <path>)

Writes:
  .autospec/reports/verifier-report.json
  .autospec/reports/verifier-report.md
  .autospec/state/verifications/<issue-or-pr-id>.json
  .autospec/state/verifications/<issue-or-pr-id>.md
EOF
}

die() {
    printf 'autospec-verify-worker-pr: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
CONFIRM=0
GH_REPO=""
ISSUE=""
PR=""
WORK_ITEM=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        --repo) [ "$#" -ge 2 ] || die "--repo requires OWNER/REPO"; GH_REPO="$2"; shift 2 ;;
        --issue) [ "$#" -ge 2 ] || die "--issue requires a value"; ISSUE="$2"; shift 2 ;;
        --pr) [ "$#" -ge 2 ] || die "--pr requires a value"; PR="$2"; shift 2 ;;
        --work-item) [ "$#" -ge 2 ] || die "--work-item requires a value"; WORK_ITEM="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$CONFIRM" "$GH_REPO" "$ISSUE" "$PR" "$WORK_ITEM" <<'PY'
import json
import os
import re
import subprocess
import sys
import tempfile

repo_root = os.path.realpath(sys.argv[1])
confirm = sys.argv[2] == "1"
gh_repo = sys.argv[3]
issue_arg = sys.argv[4]
pr_arg = sys.argv[5]
work_item_arg = sys.argv[6]

autospec_dir = os.path.join(repo_root, ".autospec")
reports_dir = os.path.join(autospec_dir, "reports")
state_dir = os.path.join(autospec_dir, "state")
verifications_dir = os.path.join(state_dir, "verifications")
report_json = os.path.join(reports_dir, "verifier-report.json")
report_md = os.path.join(reports_dir, "verifier-report.md")

REQUIRED_PR_SECTIONS = [
    "Summary",
    "Source issue",
    "Constitution/baseline references",
    "Implementation mode",
    "Files changed",
    "Validation",
    "Evidence artifacts",
    "Safety notes",
    "Follow-up issues",
]
CODE_PR_SECTIONS = ["Risk classification", "Patch budget", "Test-first plan", "Diff safety review"]
DIMENSIONS = [
    "issue_alignment",
    "acceptance_criteria",
    "constitution_alignment",
    "baseline_alignment",
    "risk_classification",
    "patch_budget",
    "forbidden_paths",
    "test_evidence",
    "validation_evidence",
    "documentation_sync",
    "metadata_sync",
    "pr_body_completeness",
    "human_readability",
    "stuck_or_followup_handling",
]
HIGH_RISK_PATHS = ["auth", "migration", ".env", "secret", "credential", ".github/workflows", "package.json", "go.mod", "pyproject.toml"]


def load_json(path, default):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            data = json.load(fh)
        return data if isinstance(data, dict) else default
    except Exception:
        return default


def read_text(path):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return fh.read()
    except Exception:
        return ""


def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
        fh.write("\n")


def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text.rstrip() + "\n")


def gh_base():
    base = ["gh"]
    if gh_repo:
        base.extend(["--repo", gh_repo])
    return base


def run_gh(args):
    completed = subprocess.run(gh_base() + args, cwd=repo_root, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr.strip() or completed.stdout.strip())
    return completed.stdout.strip()


def md_section_exists(body, name):
    return re.search(rf"^##+\s+{re.escape(name)}\s*$", body or "", re.I | re.M) is not None


def dimension(name, status, summary, evidence=None, required_action=""):
    return {
        "dimension": name,
        "status": status,
        "summary": summary,
        "evidence": evidence or [],
        "required_action": required_action,
    }


def parse_acceptance(text):
    criteria = []
    in_section = False
    for line in (text or "").splitlines():
        if re.match(r"^##+\s+Acceptance criteria\s*$", line, re.I):
            in_section = True
            continue
        if in_section and line.startswith("##"):
            break
        if in_section:
            match = re.match(r"^\s*-\s+\[[ xX]\]\s+(.+)$", line)
            if match:
                criteria.append(match.group(1).strip())
    return criteria


def criterion_status(criterion, evidence_text):
    words = [word.lower() for word in re.findall(r"[A-Za-z0-9_/-]{4,}", criterion)]
    hits = [word for word in words if word in evidence_text.lower()]
    if len(hits) >= max(1, min(2, len(words))):
        return "satisfied", hits
    if hits:
        return "partially_satisfied", hits
    return "unknown", []


def source_issue_from_plan(processed_issue_id):
    plan = load_json(os.path.join(reports_dir, "issue-plan.json"), {})
    for item in plan.get("issues", []) if isinstance(plan.get("issues"), list) else []:
        if not processed_issue_id or item.get("issue_id") == processed_issue_id:
            return item
    return {}


def local_issue_body(issue):
    path = issue.get("draft_path", "")
    return read_text(os.path.join(repo_root, path)) if path else ""


def load_pr():
    if not pr_arg:
        return {}
    pr = json.loads(run_gh(["pr", "view", pr_arg, "--json", "title,body,labels,headRefName,files,statusCheckRollup"]))
    try:
        pr["diff"] = run_gh(["pr", "diff", pr_arg])
    except Exception:
        pr["diff"] = ""
    return pr


risk = load_json(os.path.join(reports_dir, "worker-risk-classification.json"), {})
validation_plan = load_json(os.path.join(reports_dir, "worker-validation-plan.json"), {})
validation = load_json(os.path.join(reports_dir, "worker-validation.json"), {})
diff_review = load_json(os.path.join(reports_dir, "worker-diff-review.json"), {})
worker_result = load_json(os.path.join(reports_dir, "worker-result.json"), {})
baseline_comp = load_json(os.path.join(reports_dir, "baseline-composition.json"), {})
baseline_gap = load_json(os.path.join(reports_dir, "baseline-gap-analysis.json"), {})
constitutional_gap = load_json(os.path.join(reports_dir, "constitutional-gap-report.json"), {})
rule_checks = load_json(os.path.join(reports_dir, "rule-check-results.json"), {})
packet_md = read_text(os.path.join(state_dir, "implementation-packet.md"))
worker_pr_body = read_text(os.path.join(reports_dir, "worker-pr-body.md"))
pr = load_pr()
pr_body = pr.get("body") or worker_pr_body or packet_md
classification = risk.get("classification", "unknown")
is_code_change = classification in {"low-risk-code", "medium-risk-code", "high-risk-code", "architecture-required"}
processed_issue_id = risk.get("processed_issue_id", "")
source_issue = source_issue_from_plan(processed_issue_id)
issue_body = local_issue_body(source_issue)
changed_files = [item.get("path", "") for item in diff_review.get("files_changed", [])]
if pr.get("files"):
    changed_files = [item.get("path", "") for item in pr.get("files", []) if item.get("path")]

dimensions = []

alignment_evidence = []
if processed_issue_id:
    alignment_evidence.append(f"worker issue: {processed_issue_id}")
if source_issue:
    alignment_evidence.append("source issue found in issue-plan.json")
if processed_issue_id and processed_issue_id in (pr_body + packet_md + issue_body):
    dimensions.append(dimension("issue_alignment", "pass", "PR/packet links to the processed source issue.", alignment_evidence))
elif processed_issue_id and source_issue and not pr_arg:
    dimensions.append(dimension("issue_alignment", "pass", "Local work item aligns with issue-plan entry.", alignment_evidence))
elif processed_issue_id and source_issue:
    dimensions.append(dimension("issue_alignment", "warn", "Source issue exists but PR linkage is unclear.", alignment_evidence, "Add source issue linkage to the PR body."))
else:
    dimensions.append(dimension("issue_alignment", "fail", "Source issue alignment is missing or mismatched.", alignment_evidence, "Link the PR to the Autospec issue or local issue draft."))

criteria = parse_acceptance(issue_body) or list(source_issue.get("acceptance_criteria", [])) or parse_acceptance(packet_md) or parse_acceptance(pr_body)
criterion_rows = []
evidence_text = "\n".join([pr_body, packet_md, json.dumps(validation), json.dumps(validation_plan)])
for criterion in criteria:
    status, hits = criterion_status(criterion, evidence_text)
    criterion_rows.append({"criterion": criterion, "evidence": hits, "status": status, "required_action": "" if status == "satisfied" else "Add concrete evidence for this acceptance criterion."})
if not criterion_rows:
    dimensions.append(dimension("acceptance_criteria", "unknown", "No acceptance criteria were found.", [], "Add acceptance criteria to the issue or packet."))
else:
    failed_ac = [row for row in criterion_rows if row["status"] in {"not_satisfied", "unknown"}]
    dimensions.append(dimension("acceptance_criteria", "warn" if failed_ac else "pass", "Acceptance criteria were reviewed with simple evidence matching.", [row["criterion"] for row in criterion_rows], "Add missing AC evidence." if failed_ac else ""))

trace_text = "\n".join([pr_body, packet_md])
constitution_ok = bool(re.search(r"constitution|doctrine|quality gate", trace_text, re.I))
baseline_ok = bool(re.search(r"baseline|source gap|pack", trace_text, re.I))
dimensions.append(dimension("constitution_alignment", "pass" if constitution_ok else "fail", "Constitution/doctrine traceability is present." if constitution_ok else "Constitution/doctrine traceability is missing.", ["constitutional-gap-report.json" if constitutional_gap else ""]))
dimensions.append(dimension("baseline_alignment", "pass" if baseline_ok else "fail", "Baseline/source-gap traceability is present." if baseline_ok else "Baseline/source-gap traceability is missing.", ["baseline-composition.json" if baseline_comp else "", "baseline-gap-analysis.json" if baseline_gap else ""]))

rule_ids = sorted(set(re.findall(r"\b[a-z][a-z0-9_]*(?:\.[a-z0-9_]+){2,}\b", "\n".join([pr_body, packet_md, issue_body, json.dumps(source_issue)]))))
rule_known = {item.get("rule_id") for item in rule_checks.get("results", []) if isinstance(item, dict)}
rule_evidence = [rid for rid in rule_ids if not rule_known or rid in rule_known]
managed_issue = "autospec:managed" in source_issue.get("suggested_labels", []) or "autospec:managed" in pr_body
rule_context_available = bool(rule_known) or os.path.isfile(os.path.join(reports_dir, "issue-plan-v2.json"))
if rule_evidence:
    dimensions.append(dimension("rule_traceability", "pass", "Autospec rule IDs are referenced by the worker/PR evidence.", rule_evidence))
elif rule_context_available and (managed_issue or source_issue):
    dimensions.append(dimension("rule_traceability", "warn", "Autospec-generated work does not reference a source rule ID.", [], "Add the source rule ID from issue-plan-v2 or rule-check-results."))
else:
    dimensions.append(dimension("rule_traceability", "pass", "No rule-audit context was available for this v0 verification path.", []))

if classification == "unknown":
    dimensions.append(dimension("risk_classification", "fail", "Worker risk classification is missing.", [], "Run worker v1 before verification."))
elif classification == "low-risk-code" and any(any(token in path.lower() for token in HIGH_RISK_PATHS) for path in changed_files):
    dimensions.append(dimension("risk_classification", "fail", "Actual diff touches high-risk paths despite low-risk classification.", changed_files, "Reclassify or split the issue."))
else:
    dimensions.append(dimension("risk_classification", "pass", f"Risk classification exists: {classification}.", risk.get("classification_reasons", [])))

patch_budget = diff_review.get("patch_budget", {})
budget_passed = patch_budget.get("passed") is True
dimensions.append(dimension("patch_budget", "pass" if budget_passed else "fail", "Patch budget passed." if budget_passed else "Patch budget failed.", patch_budget.get("failures", []), "Reduce scope or split the issue." if not budget_passed else ""))

forbidden = diff_review.get("forbidden_path_check", {})
forbidden_passed = forbidden.get("passed") is True
dimensions.append(dimension("forbidden_paths", "pass" if forbidden_passed else "fail", "No forbidden paths touched." if forbidden_passed else "Forbidden paths were touched.", [json.dumps(item, sort_keys=True) for item in forbidden.get("matches", [])], "Remove forbidden path changes."))

test_info = diff_review.get("test_docs_metadata_change_check", {})
has_tests = bool(test_info.get("test_files") or validation_plan.get("focused_validation"))
if is_code_change and not has_tests:
    dimensions.append(dimension("test_evidence", "fail", "Code changes lack focused test evidence.", [], "Add or cite a focused test."))
else:
    dimensions.append(dimension("test_evidence", "pass" if has_tests else "warn", "Test evidence exists." if has_tests else "No focused test evidence was needed for non-code work.", test_info.get("test_files", [])))

validation_exists = bool(validation)
validation_failed = any((item.get("exit_code") not in {0, "0", None}) for key in ["focused", "full"] for item in validation.get(key, []) if isinstance(item, dict))
if is_code_change and not validation_exists:
    dimensions.append(dimension("validation_evidence", "fail", "Code changes are missing captured validation results.", [], "Run focused validation and capture exit codes."))
elif validation_failed:
    dimensions.append(dimension("validation_evidence", "fail", "Validation failures are present.", [json.dumps(validation, sort_keys=True)], "Fix failures or document a blocker."))
else:
    dimensions.append(dimension("validation_evidence", "pass" if validation_exists else "warn", "Validation evidence is captured." if validation_exists else "Validation evidence is light but acceptable for non-code work.", [json.dumps(validation, sort_keys=True)] if validation else validation_plan.get("skipped_validation", [])))

user_visible = any(path.startswith("src/") for path in changed_files)
docs_changed = any(path.startswith("docs/") or path.endswith(".md") for path in changed_files)
dimensions.append(dimension("documentation_sync", "pass" if docs_changed or not user_visible else "warn", "Docs sync appears adequate." if docs_changed or not user_visible else "User-visible/helper behavior changed without docs evidence.", changed_files, "Update docs or explain why docs are not needed." if user_visible and not docs_changed else ""))

metadata_changed = any(path.startswith(".autospec/") for path in changed_files)
metadata_referenced = ".autospec/reports" in pr_body or ".autospec/state" in pr_body or bool(worker_result)
metadata_ok = metadata_changed or metadata_referenced or not baseline_gap or not is_code_change
dimensions.append(dimension("metadata_sync", "pass" if metadata_ok else "warn", "Metadata sync checked.", changed_files, "Reference generated Autospec reports where useful." if not metadata_ok else ""))

if pr_arg:
    required_sections = list(REQUIRED_PR_SECTIONS)
    if is_code_change:
        required_sections.extend(CODE_PR_SECTIONS)
    missing_sections = [section for section in required_sections if not md_section_exists(pr_body, section)]
    dimensions.append(dimension("pr_body_completeness", "pass" if not missing_sections else "fail", "PR body contains required sections." if not missing_sections else "PR body is missing required sections.", missing_sections, "Add missing PR sections." if missing_sections else ""))
else:
    packet_sections = ["Risk classification", "Test-first plan"]
    missing_sections = [section for section in packet_sections if not md_section_exists(packet_md, section)]
    dimensions.append(dimension("pr_body_completeness", "pass" if not missing_sections else "warn", "Implementation packet contains the required local-work sections." if not missing_sections else "Implementation packet is missing useful sections.", missing_sections))

readable = bool(pr_body.strip()) and ("##" in pr_body)
dimensions.append(dimension("human_readability", "pass" if readable else "warn", "Human-facing Markdown is readable and table-oriented." if readable else "Human-facing report needs clearer Markdown/table structure.", []))

followup_ok = (not pr_arg) or md_section_exists(pr_body, "Follow-up issues") or "stuck" in pr_body.lower() or os.path.isfile(os.path.join(reports_dir, "worker-stuck-handoff.md"))
dimensions.append(dimension("stuck_or_followup_handling", "pass" if followup_ok else "warn", "Follow-up or stuck handling is documented." if followup_ok else "Follow-up/stuck handling is missing.", []))

statuses = {item["dimension"]: item["status"] for item in dimensions}
if statuses.get("forbidden_paths") == "fail" or statuses.get("risk_classification") == "fail":
    verdict = "blocked"
elif statuses.get("forbidden_paths") == "fail":
    verdict = "blocked"
elif statuses.get("issue_alignment") == "fail":
    verdict = "needs_guidance"
elif any(item["dimension"] == "issue_alignment" and item["status"] == "fail" for item in dimensions):
    verdict = "needs_guidance"
elif any(item["status"] == "fail" for item in dimensions):
    verdict = "needs_changes"
elif any(item["status"] in {"warn", "unknown"} for item in dimensions):
    verdict = "pass_with_warnings"
else:
    verdict = "pass"
if statuses.get("patch_budget") == "fail" and statuses.get("forbidden_paths") == "fail":
    verdict = "blocked"

required_actions = [item["required_action"] for item in dimensions if item.get("required_action")]
source_id = f"pr-{pr_arg}" if pr_arg else f"issue-{issue_arg}" if issue_arg else "work-item"
state_json = os.path.join(verifications_dir, f"{source_id}.json")
state_md = os.path.join(verifications_dir, f"{source_id}.md")

report = {
    "version": 1,
    "mode": "confirm" if confirm else "dry_run",
    "verdict": verdict,
    "source": {"issue": issue_arg, "pr": pr_arg, "work_item": work_item_arg, "processed_issue_id": processed_issue_id},
    "dimensions": dimensions,
    "acceptance_criteria": criterion_rows,
    "required_actions": required_actions,
    "side_effects": {"github_comment": bool(confirm and pr_arg), "approved": False, "merged": False, "pushed": False},
}
write_json(report_json, report)
write_json(state_json, report)

matrix = "\n".join(f"| {item['dimension']} | {item['status']} | {item['summary']} | {item.get('required_action','')} |" for item in dimensions)
ac_table = "\n".join(f"| {row['criterion']} | {', '.join(row['evidence']) or 'none'} | {row['status']} | {row['required_action']} |" for row in criterion_rows) or "| none | none | unknown | Add acceptance criteria. |"
findings = [item for item in dimensions if item["status"] in {"fail", "warn", "unknown"}]
findings_text = "\n".join(f"- **{item['dimension']}**: {item['summary']}" for item in findings) or "- None."
required_text = "\n".join(f"- {action}" for action in required_actions) or "- None."
md = "\n".join([
    "# Autospec Verifier Report",
    "",
    "## Verdict",
    "",
    f"**{verdict}**",
    "",
    "## Summary",
    "",
    f"Reviewed worker output for `{processed_issue_id or source_id}`. The verifier did not approve, merge, push, or modify code.",
    "",
    "## Source issue / PR",
    "",
    f"- Issue: `{issue_arg or processed_issue_id or 'unknown'}`",
    f"- PR: `{pr_arg or 'none'}`",
    "",
    "## Verification matrix",
    "",
    "| Dimension | Status | Summary | Required action |",
    "| --- | --- | --- | --- |",
    matrix,
    "",
    "## Acceptance criteria review",
    "",
    "| Criterion | Evidence | Status | Required action |",
    "| --- | --- | --- | --- |",
    ac_table,
    "",
    "## Risk and patch budget",
    "",
    f"- Classification: `{classification}`",
    f"- Patch budget: `{statuses.get('patch_budget', 'unknown')}`",
    f"- Forbidden paths: `{statuses.get('forbidden_paths', 'unknown')}`",
    "",
    "## Validation evidence",
    "",
    f"- Validation status: `{statuses.get('validation_evidence', 'unknown')}`",
    "",
    "## Docs/metadata sync",
    "",
    f"- Documentation: `{statuses.get('documentation_sync', 'unknown')}`",
    f"- Metadata: `{statuses.get('metadata_sync', 'unknown')}`",
    "",
    "## Findings",
    "",
    findings_text,
    "",
    "## Required actions",
    "",
    required_text,
    "",
    "## Recommended next step",
    "",
    "Address required actions, rerun the worker validation, then rerun this verifier.",
])
write_text(report_md, md)
write_text(state_md, md)

if confirm and pr_arg:
    comment = "\n".join([
        "## Autospec verifier result",
        "",
        f"**Verdict:** {verdict}",
        "",
        "### Summary",
        "",
        f"Reviewed worker output for `{processed_issue_id or source_id}`.",
        "",
        "### Required actions",
        "",
        required_text,
        "",
        "### Evidence",
        "",
        "- verifier report: `.autospec/reports/verifier-report.md`",
    ])
    fd, path = tempfile.mkstemp(prefix="autospec-verifier-comment-", suffix=".md")
    with os.fdopen(fd, "w", encoding="utf-8") as fh:
        fh.write(comment)
        fh.write("\n")
    try:
        run_gh(["pr", "comment", pr_arg, "--body-file", path])
    finally:
        try:
            os.unlink(path)
        except OSError:
            pass

print(f"verifier: {verdict}")
print("reports: .autospec/reports/verifier-report.json, .autospec/reports/verifier-report.md")
sys.exit(0 if verdict in {"pass", "pass_with_warnings"} else 1)
PY

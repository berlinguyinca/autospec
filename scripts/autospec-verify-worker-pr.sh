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
from pathlib import Path

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


def repo_has_architecture_evidence(repo_root):
    evidence = []
    for base, dirs, files in os.walk(repo_root):
        rel_base = os.path.relpath(base, repo_root)
        if rel_base == ".":
            rel_base = ""
        parts = set(Path(rel_base).parts)
        if {".git", "node_modules", "__pycache__"} & parts:
            dirs[:] = []
            continue
        if rel_base.startswith(".autospec/templates"):
            dirs[:] = []
            continue
        for name in files:
            rel_path = os.path.join(rel_base, name).strip(os.sep)
            low = rel_path.lower()
            if re.search(r"(^|/)adr[s]?/|architecture.*\.md|decision", low):
                evidence.append(rel_path)
    return sorted(evidence)


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
    for filename in ["issue-plan-v3.json", "issue-plan-v2.json", "issue-plan.json"]:
        plan = load_json(os.path.join(reports_dir, filename), {})
        for item in plan.get("issues", []) if isinstance(plan.get("issues"), list) else []:
            if not processed_issue_id or item.get("issue_id") == processed_issue_id:
                found = dict(item)
                found["plan_version"] = "v3" if filename.endswith("v3.json") else "v2" if filename.endswith("v2.json") else "v1"
                return found
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
worker_recipe_execution = load_json(os.path.join(reports_dir, "worker-recipe-execution.json"), {})
runtime_generation = load_json(os.path.join(reports_dir, "runtime-generation-result.json"), {})
runtime_verification = load_json(os.path.join(reports_dir, "runtime-feature-verification.json"), {})
runtime_metadata_sync = load_json(os.path.join(reports_dir, "runtime-metadata-sync.json"), {})
playwright_generation = load_json(os.path.join(reports_dir, "playwright-generation-result.json"), {})
playwright_evidence_run = load_json(os.path.join(reports_dir, "playwright-evidence-run.json"), {})
screenshot_contact_sheet = load_json(os.path.join(reports_dir, "screenshot-contact-sheet.json"), {})
visual_polish_audit = load_json(os.path.join(reports_dir, "visual-polish-audit.json"), {})
accessibility_evidence_audit = load_json(os.path.join(reports_dir, "accessibility-evidence-audit.json"), {})
tutorial_artifacts = load_json(os.path.join(reports_dir, "tutorial-artifacts.json"), {})
pdf_artifact_plan = load_json(os.path.join(reports_dir, "pdf-artifact-plan.json"), {})
report_artifact_generation = load_json(os.path.join(reports_dir, "report-artifact-generation.json"), {})
ai_nlai_simulation = load_json(os.path.join(reports_dir, "ai-nlai-simulation.json"), {})
token_usage_evidence = load_json(os.path.join(reports_dir, "token-usage-evidence.json"), {})
evidence_bundle = load_json(os.path.join(reports_dir, "evidence-bundle.json"), {})
patch_plan_data = load_json(os.path.join(reports_dir, "patch-plan.json"), {})
stack_profile = load_json(os.path.join(state_dir, "stack-profile.json"), load_json(os.path.join(reports_dir, "stack-profile.json"), {}))
rule_recheck = load_json(os.path.join(reports_dir, "rule-recheck.json"), {})
baseline_comp = load_json(os.path.join(reports_dir, "baseline-composition.json"), {})
baseline_gap = load_json(os.path.join(reports_dir, "baseline-gap-analysis.json"), {})
constitutional_gap = load_json(os.path.join(reports_dir, "constitutional-gap-report.json"), {})
rule_checks = load_json(os.path.join(reports_dir, "rule-check-results.json"), {})
work_item_packet_json = os.path.join(work_item_arg, "implementation-packet.json") if work_item_arg else ""
work_item_packet_md = os.path.join(work_item_arg, "implementation-packet.md") if work_item_arg else ""
packet_data = load_json(work_item_packet_json, load_json(os.path.join(state_dir, "implementation-packet.json"), {}))
packet_md = read_text(work_item_packet_md) or read_text(os.path.join(state_dir, "implementation-packet.md"))
work_item_rule_progress_json = os.path.join(work_item_arg, "rule-progress.json") if work_item_arg else ""
rule_progress = load_json(work_item_rule_progress_json, load_json(os.path.join(reports_dir, "worker-rule-progress.json"), {}))
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

policy_context = packet_data.get("structured_policy_context", {}) if isinstance(packet_data.get("structured_policy_context"), dict) else {}
trace_text = "\n".join([pr_body, packet_md, json.dumps(policy_context, sort_keys=True), json.dumps(source_issue, sort_keys=True)])
constitution_ok = bool(re.search(r"constitution|doctrine|quality gate", trace_text, re.I))
baseline_ok = bool(re.search(r"baseline|source gap|pack", trace_text, re.I)) or bool(policy_context.get("source_baseline_pack") or source_issue.get("source_baseline_pack"))
dimensions.append(dimension("constitution_alignment", "pass" if constitution_ok else "fail", "Constitution/doctrine traceability is present." if constitution_ok else "Constitution/doctrine traceability is missing.", ["constitutional-gap-report.json" if constitutional_gap else ""]))
dimensions.append(dimension("baseline_alignment", "pass" if baseline_ok else "fail", "Baseline/source-gap traceability is present." if baseline_ok else "Baseline/source-gap traceability is missing.", ["baseline-composition.json" if baseline_comp else "", "baseline-gap-analysis.json" if baseline_gap else ""]))

rule_ids = sorted(set((policy_context.get("rule_ids") or []) + re.findall(r"\b[a-z][a-z0-9_]*(?:\.[a-z0-9_]+){2,}\b", "\n".join([pr_body, packet_md, issue_body, json.dumps(source_issue)]))))
quality_gate_ids = sorted(set(policy_context.get("quality_gate_ids") or source_issue.get("quality_gate_ids", []) or []))
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

v3_context_expected = source_issue.get("plan_version") == "v3" or bool(policy_context.get("rule_ids"))
if v3_context_expected:
    policy_status = "pass" if rule_ids and bool(policy_context) else "warn"
    policy_summary = "Structured policy context is traceable through the packet." if policy_status == "pass" else "Structured policy context is missing or incomplete."
else:
    policy_status = "pass"
    policy_summary = "Structured policy context is not required for this legacy verification path."
policy_traceability = {
    "status": policy_status,
    "rule_ids": rule_ids,
    "quality_gate_ids": quality_gate_ids,
    "summary": policy_summary,
    "missing": [] if policy_status == "pass" else ["structured_policy_context.rule_ids"],
}
dimensions.append(dimension("policy_traceability", policy_status, policy_summary, rule_ids, "Add structured policy context to the implementation packet." if policy_status != "pass" else ""))
quality_gate_review = {
    "status": "pass" if quality_gate_ids or not v3_context_expected else "warn",
    "quality_gate_ids": quality_gate_ids,
    "summary": "Quality gates are referenced." if quality_gate_ids else "No quality gates were referenced.",
}
maturity_impact = {
    "status": "known" if policy_context.get("maturity_target") or source_issue.get("maturity_level") else "unknown",
    "maturity_target": policy_context.get("maturity_target") or source_issue.get("maturity_level", ""),
    "category": policy_context.get("category") or source_issue.get("category", ""),
    "severity": policy_context.get("severity") or source_issue.get("rule_severity") or source_issue.get("severity", ""),
}
rule_progress_required = v3_context_expected
rule_progress_rows = []
rule_progress_status = "not_required"
if rule_progress_required and not rule_progress.get("rule_ids"):
    rule_progress_status = "missing"
elif rule_progress_required:
    before_by_id = {item.get("rule_id"): item for item in rule_progress.get("before", []) if isinstance(item, dict)}
    after_by_id = {item.get("rule_id"): item for item in rule_progress.get("after", []) if isinstance(item, dict)}
    row_failures = []
    for rid in rule_progress.get("rule_ids", []):
        before_status = before_by_id.get(rid, {}).get("status", "unknown")
        after_status = after_by_id.get(rid, {}).get("status", "unknown")
        evidence = after_by_id.get(rid, {}).get("evidence", [])
        row_status = "pass"
        if after_status == "pass" and not evidence:
            row_status = "fail"
        elif after_status in {"fail", "unknown"}:
            row_status = "warn"
        elif after_status == "partial":
            row_status = "warn"
        if re.search(r"\b(rule|constitutional|compliance)\b.{0,40}\b(complete|fixed)\b", pr_body, re.I | re.S) and after_status != "pass":
            row_status = "fail"
        if row_status == "fail":
            row_failures.append(rid)
        rule_progress_rows.append({
            "rule_id": rid,
            "before": before_status,
            "after": after_status,
            "evidence": evidence,
            "verifier_status": row_status,
        })
    rule_progress_status = "fail" if row_failures else "warn" if any(row["verifier_status"] == "warn" for row in rule_progress_rows) else "pass"
else:
    rule_progress_rows = []
rule_progress_verification = {
    "status": rule_progress_status,
    "required": rule_progress_required,
    "rows": rule_progress_rows,
    "summary": "Rule progress evidence was reviewed." if rule_progress_status not in {"missing", "not_required"} else "Rule progress evidence is missing." if rule_progress_status == "missing" else "Rule progress evidence is not required for this legacy path.",
}
if rule_progress_required:
    dimensions.append(dimension(
        "rule_progress",
        "fail" if rule_progress_status == "fail" else "warn" if rule_progress_status in {"missing", "warn"} else "pass",
        rule_progress_verification["summary"],
        [row["rule_id"] for row in rule_progress_rows],
        "Run worker v2/rule-aware worker and include rule-progress evidence." if rule_progress_status in {"missing", "fail"} else "",
    ))

if classification == "unknown":
    dimensions.append(dimension("risk_classification", "fail", "Worker risk classification is missing.", [], "Run worker v1 before verification."))
elif classification == "low-risk-code" and any(any(token in path.lower() for token in HIGH_RISK_PATHS) for path in changed_files):
    dimensions.append(dimension("risk_classification", "fail", "Actual diff touches high-risk paths despite low-risk classification.", changed_files, "Reclassify or split the issue."))
else:
    dimensions.append(dimension("risk_classification", "pass", f"Risk classification exists: {classification}.", risk.get("classification_reasons", [])))

architecture_expected = (
    classification in {"architecture-required", "high-risk-code"}
    or "autospec:architecture" in source_issue.get("suggested_labels", [])
    or "autospec:risk-high" in source_issue.get("suggested_labels", [])
    or (isinstance(source_issue.get("risk"), dict) and source_issue.get("risk", {}).get("level") == "high")
)
if architecture_expected:
    arch_report = load_json(os.path.join(reports_dir, "architecture-governance.json"), {})
    arch_checks = arch_report.get("checks", {}) if isinstance(arch_report.get("checks"), dict) else {}
    arch_evidence = []
    for key in ["adrs", "architecture_map", "design_pattern_rationale", "impact_analysis"]:
        item = arch_checks.get(key, {}) if isinstance(arch_checks.get(key), dict) else {}
        arch_evidence.extend(item.get("evidence", []) if isinstance(item.get("evidence"), list) else [])
    if not arch_evidence:
        arch_evidence = repo_has_architecture_evidence(repo_root)
    dimensions.append(dimension(
        "architecture_governance",
        "pass" if arch_evidence else "warn",
        "Architecture Governance evidence exists." if arch_evidence else "Architecture Governance evidence is missing for architecture/high-risk work.",
        arch_evidence,
        "Add an ADR, architecture notes, design-pattern rationale, or impact analysis before promotion." if not arch_evidence else "",
    ))

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

recipe_review = {"status": "not_required", "summary": "No recipe execution artifact was provided.", "evidence": []}
patch_plan_compliance = {"status": "not_required", "summary": "No recipe patch plan was provided.", "evidence": []}
template_application_review = {"status": "not_required", "summary": "No template application artifact was provided.", "evidence": []}
stack_profile_review = {"status": "not_required", "summary": "No stack-specific recipe execution was provided.", "evidence": []}
rule_recheck_review = {"status": "not_required", "summary": "No rule recheck artifact was provided.", "evidence": []}
scaffold_honesty = {"status": "pass", "summary": "No scaffolded work is being claimed as complete runtime implementation.", "evidence": []}
runtime_feature_review = {"status": "not_required", "summary": "No runtime feature generation artifact was provided.", "evidence": []}
adapter_compliance = {"status": "not_required", "summary": "No runtime adapter was used.", "evidence": []}
generated_files_review = {"status": "not_required", "summary": "No runtime files were generated.", "evidence": []}
runtime_claim_honesty = {"status": "not_required", "summary": "No runtime claim was made.", "evidence": []}
ui_ux_evidence = {"status": "not_required", "summary": "No UI runtime shell was generated.", "evidence": []}
playwright_evidence_review = {"status": "not_required", "summary": "No Playwright runtime evidence was provided.", "evidence": []}
metadata_synchronization = {"status": "not_required", "summary": "No runtime metadata sync artifact was provided.", "evidence": []}
evidence_bundle_review = {"status": "not_required", "summary": "No runtime/UI/AI/reporting evidence bundle was required.", "evidence": []}
screenshot_evidence_review = {"status": "not_required", "summary": "No screenshot evidence was required.", "evidence": []}
accessibility_evidence_review = {"status": "not_required", "summary": "No accessibility evidence was required.", "evidence": []}
tutorial_pdf_report_evidence = {"status": "not_required", "summary": "No tutorial/PDF/report evidence was required.", "evidence": []}
ai_nlai_simulation_evidence = {"status": "not_required", "summary": "No AI/NLAI simulation evidence was required.", "evidence": []}
token_usage_evidence_review = {"status": "not_required", "summary": "No token usage evidence was required.", "evidence": []}
evidence_gaps = {"status": "pass", "summary": "No evidence gaps detected.", "evidence": []}

if worker_recipe_execution:
    recipe_id = worker_recipe_execution.get("recipe_id") or worker_recipe_execution.get("recipe", {}).get("id", "")
    capability = worker_recipe_execution.get("capability", "")
    execution_status = worker_recipe_execution.get("status", "unknown")
    recipe_review = {
        "status": "pass" if execution_status in {"executed", "planned", "dry_run"} else "warn",
        "summary": f"Recipe execution artifact reviewed for `{recipe_id or 'unknown'}`.",
        "evidence": [f"status={execution_status}", f"capability={capability or 'unknown'}"],
    }
    dimensions.append(dimension("recipe_review", recipe_review["status"], recipe_review["summary"], recipe_review["evidence"]))

    plan_recipe = patch_plan_data.get("recipe_id") or patch_plan_data.get("recipe", {}).get("id", "")
    patch_status = "pass" if patch_plan_data and (not recipe_id or not plan_recipe or plan_recipe == recipe_id) else "warn"
    patch_plan_compliance = {
        "status": patch_status,
        "summary": "Patch plan exists and is tied to recipe execution." if patch_status == "pass" else "Patch plan is missing or does not match recipe execution.",
        "evidence": [f"recipe={recipe_id or 'unknown'}", f"patch_plan_recipe={plan_recipe or 'unknown'}"],
    }
    dimensions.append(dimension("patch_plan_compliance", patch_status, patch_plan_compliance["summary"], patch_plan_compliance["evidence"], "Regenerate the patch plan from the selected recipe." if patch_status != "pass" else ""))

    template_application = load_json(os.path.join(reports_dir, "template-apply-result.json"), {})
    unsafe_overwrite = bool(template_application.get("unsafe_overwrite"))
    template_application_review = {
        "status": "fail" if unsafe_overwrite else "pass",
        "summary": "Template application did not report unsafe overwrites." if not unsafe_overwrite else "Template application reported an unsafe overwrite.",
        "evidence": [json.dumps(template_application, sort_keys=True)] if template_application else ["no template application result"],
    }
    dimensions.append(dimension("template_application_review", template_application_review["status"], template_application_review["summary"], template_application_review["evidence"], "Do not overwrite non-generated files." if unsafe_overwrite else ""))

    primary_stack = stack_profile.get("primary_profile") or {}
    confidence = float(primary_stack.get("confidence", 0) or 0)
    stack_specific = capability in {"ui_scaffold", "api_scaffold", "settings_scaffold"}
    stack_status = "pass" if (not stack_specific or confidence >= 0.8) else "fail"
    stack_profile_review = {
        "status": stack_status,
        "summary": "Stack confidence is sufficient for the selected recipe." if stack_status == "pass" else "Stack-specific scaffold ran without sufficient stack confidence.",
        "evidence": [f"stack={primary_stack.get('id', 'unknown')}", f"confidence={confidence:.2f}", f"capability={capability or 'unknown'}"],
    }
    dimensions.append(dimension("stack_profile_review", stack_status, stack_profile_review["summary"], stack_profile_review["evidence"], "Use spec-only scaffold or add human guidance for uncertain stacks." if stack_status != "pass" else ""))

    recheck_results = rule_recheck.get("results", [])
    recheck_status = "pass" if recheck_results else "warn"
    rule_recheck_review = {
        "status": recheck_status,
        "summary": "Rule recheck evidence is present." if recheck_results else "Rule recheck evidence is missing or skipped.",
        "evidence": [json.dumps(recheck_results[:3], sort_keys=True)] if recheck_results else [rule_recheck.get("skip_rationale", "missing")],
    }
    dimensions.append(dimension("rule_recheck_review", recheck_status, rule_recheck_review["summary"], rule_recheck_review["evidence"], "Run autospec-rule-recheck for recipe-backed work or document a skip rationale." if recheck_status != "pass" else ""))

    recipe_mode = worker_recipe_execution.get("mode") or worker_recipe_execution.get("implementation_mode", "")
    false_runtime_claim = recipe_mode in {"scaffold", "planning_only", "docs", "metadata", "test"} and re.search(r"\b(fully implemented|runtime complete|production-ready runtime)\b", pr_body, re.I)
    scaffold_honesty = {
        "status": "fail" if false_runtime_claim else "pass",
        "summary": "Scaffolded work is represented honestly." if not false_runtime_claim else "Scaffolded work is being claimed as complete runtime implementation.",
        "evidence": [f"mode={recipe_mode or 'unknown'}"],
    }
    dimensions.append(dimension("scaffold_honesty", scaffold_honesty["status"], scaffold_honesty["summary"], scaffold_honesty["evidence"], "Describe scaffolded output as scaffolded, not complete runtime behavior." if false_runtime_claim else ""))

if runtime_generation:
    feature_id = runtime_generation.get("feature_id", "unknown")
    runtime_status = runtime_generation.get("status", "unknown")
    runtime_feature_review = {
        "status": "pass" if runtime_status in {"planned", "generated"} else "warn",
        "summary": f"Runtime generation artifact reviewed for `{feature_id}`.",
        "evidence": [f"status={runtime_status}", f"claim={runtime_generation.get('runtime_claim_level', 'unknown')}"],
    }
    dimensions.append(dimension("runtime_feature_review", runtime_feature_review["status"], runtime_feature_review["summary"], runtime_feature_review["evidence"]))

    adapter_compliance = {
        "status": "pass" if runtime_generation.get("adapter") else "warn",
        "summary": "Runtime generation references an adapter." if runtime_generation.get("adapter") else "Runtime generation does not reference an adapter.",
        "evidence": [f"adapter={runtime_generation.get('adapter', 'none')}"],
    }
    dimensions.append(dimension("adapter_compliance", adapter_compliance["status"], adapter_compliance["summary"], adapter_compliance["evidence"]))

    generated_files = runtime_generation.get("generated_files", [])
    generated_files_review = {
        "status": "pass" if generated_files or runtime_status == "planned" else "warn",
        "summary": "Generated files are listed or this is a dry-run plan.",
        "evidence": generated_files,
    }
    dimensions.append(dimension("generated_files_review", generated_files_review["status"], generated_files_review["summary"], generated_files_review["evidence"]))

    overstated = re.search(r"\b(fully implemented|real model calls|db persistence|complete runtime)\b", pr_body, re.I) and runtime_generation.get("runtime_claim_level") in {"shell", "partial"}
    runtime_claim_honesty = {
        "status": "fail" if overstated else "pass",
        "summary": "Runtime claim is represented honestly." if not overstated else "Runtime claim is overstated.",
        "evidence": [f"claim={runtime_generation.get('runtime_claim_level', 'unknown')}"],
    }
    dimensions.append(dimension("runtime_claim_honesty", runtime_claim_honesty["status"], runtime_claim_honesty["summary"], runtime_claim_honesty["evidence"], "Describe shell/partial runtime slices honestly." if overstated else ""))

    ui_ux_evidence = {
        "status": "pass" if runtime_generation.get("runtime_claim_level") in {"shell", "partial"} else "warn",
        "summary": "UI shell includes runtime claim-level evidence.",
        "evidence": runtime_generation.get("generated_files", []),
    }
    dimensions.append(dimension("ui_ux_evidence", ui_ux_evidence["status"], ui_ux_evidence["summary"], ui_ux_evidence["evidence"]))

    playwright_evidence_review = {
        "status": "pass" if playwright_generation else "warn",
        "summary": "Playwright evidence generation report is present." if playwright_generation else "Playwright evidence generation report is missing.",
        "evidence": [json.dumps(playwright_generation, sort_keys=True)] if playwright_generation else [],
    }
    dimensions.append(dimension("playwright_evidence", playwright_evidence_review["status"], playwright_evidence_review["summary"], playwright_evidence_review["evidence"], "Generate Playwright evidence or document why unavailable." if not playwright_generation else ""))

    metadata_synchronization = {
        "status": "pass" if runtime_metadata_sync else "warn",
        "summary": "Runtime metadata sync report is present." if runtime_metadata_sync else "Runtime metadata sync report is missing.",
        "evidence": [json.dumps(runtime_metadata_sync, sort_keys=True)] if runtime_metadata_sync else [],
    }
    dimensions.append(dimension("runtime_metadata_sync", metadata_synchronization["status"], metadata_synchronization["summary"], metadata_synchronization["evidence"], "Run autospec-sync-runtime-metadata or document skip rationale." if not runtime_metadata_sync else ""))

surface_text = "\n".join([pr_body, packet_md, json.dumps(source_issue)])
evidence_required = bool(runtime_generation) or bool(re.search(
    r"\b(ui|runtime|ai|nlai|rag|dashboard|playwright|screenshot|accessibility|pdf|tutorial|reporting)\b",
    surface_text,
    re.I,
))
bundle_secret_findings = evidence_bundle.get("findings", []) if isinstance(evidence_bundle.get("findings"), list) else []
bundle_has_secret = any("secret" in str(item).lower() for item in bundle_secret_findings)
if evidence_required:
    evidence_bundle_review = {
        "status": "fail" if bundle_has_secret else "pass" if evidence_bundle else "warn",
        "summary": "Evidence bundle exists and is secret-clean." if evidence_bundle and not bundle_has_secret else "Evidence bundle contains secret-like content." if bundle_has_secret else "Evidence bundle is missing for runtime/UI/AI/reporting work.",
        "evidence": [f"bundle={bool(evidence_bundle)}", *[str(item) for item in bundle_secret_findings[:5]]],
    }
    dimensions.append(dimension("evidence_bundle_review", evidence_bundle_review["status"], evidence_bundle_review["summary"], evidence_bundle_review["evidence"], "Build an Autospec evidence bundle or remove secret-like evidence content." if evidence_bundle_review["status"] != "pass" else ""))

    screenshot_sources = []
    if playwright_evidence_run:
        screenshot_sources.extend(playwright_evidence_run.get("screenshots", []) if isinstance(playwright_evidence_run.get("screenshots"), list) else [])
    if screenshot_contact_sheet:
        screenshot_sources.extend(screenshot_contact_sheet.get("source_screenshots", []) if isinstance(screenshot_contact_sheet.get("source_screenshots"), list) else [])
    screenshot_evidence_review = {
        "status": "pass" if screenshot_sources or screenshot_contact_sheet else "warn",
        "summary": "Screenshot/contact-sheet evidence is present." if screenshot_sources or screenshot_contact_sheet else "Screenshot/contact-sheet evidence is missing or not applicable.",
        "evidence": screenshot_sources[:10],
    }
    dimensions.append(dimension("screenshot_evidence", screenshot_evidence_review["status"], screenshot_evidence_review["summary"], screenshot_evidence_review["evidence"], "Run Playwright evidence and contact-sheet generation, or document why unavailable." if screenshot_evidence_review["status"] != "pass" else ""))

    accessibility_evidence_review = {
        "status": "pass" if accessibility_evidence_audit else "warn",
        "summary": "Accessibility evidence audit exists." if accessibility_evidence_audit else "Accessibility evidence audit is missing.",
        "evidence": accessibility_evidence_audit.get("evidence", []) if isinstance(accessibility_evidence_audit.get("evidence"), list) else [],
    }
    dimensions.append(dimension("accessibility_evidence", accessibility_evidence_review["status"], accessibility_evidence_review["summary"], accessibility_evidence_review["evidence"], "Run autospec-accessibility-evidence-audit or create an adoption issue." if not accessibility_evidence_audit else ""))

    tutorial_pdf_report_evidence = {
        "status": "pass" if tutorial_artifacts or pdf_artifact_plan or report_artifact_generation else "warn",
        "summary": "Tutorial/PDF/report artifact evidence is present." if tutorial_artifacts or pdf_artifact_plan or report_artifact_generation else "Tutorial/PDF/report artifact evidence is missing or not applicable.",
        "evidence": [name for name, data in [("tutorial", tutorial_artifacts), ("pdf", pdf_artifact_plan), ("report", report_artifact_generation)] if data],
    }
    dimensions.append(dimension("tutorial_pdf_report_evidence", tutorial_pdf_report_evidence["status"], tutorial_pdf_report_evidence["summary"], tutorial_pdf_report_evidence["evidence"], "Generate tutorial, PDF, or report artifact plans for user-facing/reporting work." if tutorial_pdf_report_evidence["status"] != "pass" else ""))

    ai_text = "\n".join([pr_body, packet_md, json.dumps(runtime_generation)])
    ai_related = bool(re.search(r"\b(ai|nlai|rag|token|mcp|model|provider|assistant)\b", ai_text, re.I))
    if ai_related:
        ai_status = ai_nlai_simulation.get("status", "")
        ai_nlai_simulation_evidence = {
            "status": "pass" if ai_status in {"simulated_pass", "simulated_warn"} else "fail" if ai_related else "not_required",
            "summary": "AI/NLAI mock simulation evidence exists." if ai_status else "AI/NLAI runtime claim lacks mock simulation evidence.",
            "evidence": [f"scenario={ai_nlai_simulation.get('scenario', 'missing')}", f"status={ai_status or 'missing'}"],
        }
        dimensions.append(dimension("ai_nlai_simulation_evidence", ai_nlai_simulation_evidence["status"], ai_nlai_simulation_evidence["summary"], ai_nlai_simulation_evidence["evidence"], "Run autospec-simulate-ai-nlai with mock-only evidence before claiming AI/NLAI runtime behavior." if ai_nlai_simulation_evidence["status"] != "pass" else ""))

        token_usage_evidence_review = {
            "status": "pass" if token_usage_evidence else "warn",
            "summary": "Token usage evidence plan exists." if token_usage_evidence else "Token usage evidence plan is missing for AI-related work.",
            "evidence": token_usage_evidence.get("missing", []) if isinstance(token_usage_evidence.get("missing"), list) else [],
        }
        dimensions.append(dimension("token_usage_evidence", token_usage_evidence_review["status"], token_usage_evidence_review["summary"], token_usage_evidence_review["evidence"], "Run autospec-token-usage-evidence for AI multi-user/token-cost claims." if not token_usage_evidence else ""))

    gaps = [item["dimension"] for item in dimensions if item["dimension"] in {"evidence_bundle_review", "screenshot_evidence", "accessibility_evidence", "tutorial_pdf_report_evidence", "ai_nlai_simulation_evidence", "token_usage_evidence"} and item["status"] in {"fail", "warn", "unknown"}]
    evidence_gaps = {"status": "fail" if any(item["dimension"] == "ai_nlai_simulation_evidence" and item["status"] == "fail" for item in dimensions) or bundle_has_secret else "warn" if gaps else "pass", "summary": "Evidence gaps require reviewer attention." if gaps else "Evidence bundle review found no blocking gaps.", "evidence": gaps}

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
    "policy_traceability": policy_traceability,
    "quality_gate_review": quality_gate_review,
    "maturity_impact": maturity_impact,
    "rule_progress_verification": rule_progress_verification,
    "recipe_review": recipe_review,
    "patch_plan_compliance": patch_plan_compliance,
    "template_application_review": template_application_review,
    "stack_profile_review": stack_profile_review,
    "rule_recheck_review": rule_recheck_review,
    "scaffold_honesty": scaffold_honesty,
    "runtime_feature_review": runtime_feature_review,
    "adapter_compliance": adapter_compliance,
    "generated_files_review": generated_files_review,
    "runtime_claim_honesty": runtime_claim_honesty,
    "ui_ux_evidence": ui_ux_evidence,
    "playwright_evidence": playwright_evidence_review,
    "metadata_synchronization": metadata_synchronization,
    "evidence_bundle_review": evidence_bundle_review,
    "screenshot_evidence": screenshot_evidence_review,
    "accessibility_evidence": accessibility_evidence_review,
    "tutorial_pdf_report_evidence": tutorial_pdf_report_evidence,
    "ai_nlai_simulation_evidence": ai_nlai_simulation_evidence,
    "token_usage_evidence": token_usage_evidence_review,
    "evidence_gaps": evidence_gaps,
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
    "## Policy Traceability",
    "",
    f"- Status: `{policy_traceability['status']}`",
    f"- Rule IDs: {', '.join(f'`{rid}`' for rid in rule_ids) if rule_ids else 'none'}",
    "",
    "## Rule Compliance",
    "",
    "| Rule ID | Previous status | Expected status after PR | Evidence | Verifier status |",
    "| --- | --- | --- | --- | --- |",
    "\n".join(f"| `{rid}` | unknown | improved | packet/PR references rule | {policy_traceability['status']} |" for rid in rule_ids) or "| none | unknown | unknown | none | warn |",
    "",
    "## Quality Gate Review",
    "",
    f"- Status: `{quality_gate_review['status']}`",
    f"- Quality gates: {', '.join(f'`{gid}`' for gid in quality_gate_ids) if quality_gate_ids else 'none'}",
    "",
    "## Maturity Impact",
    "",
    f"- Target: `{maturity_impact['maturity_target'] or 'unknown'}`",
    f"- Category: `{maturity_impact['category'] or 'unknown'}`",
    f"- Severity: `{maturity_impact['severity'] or 'unknown'}`",
    "",
    "## Rule Progress Verification",
    "",
    f"- Status: `{rule_progress_verification['status']}`",
    "",
    "| Rule | Before | After | Evidence | Verifier status |",
    "| --- | --- | --- | --- | --- |",
    "\n".join(f"| `{row['rule_id']}` | {row['before']} | {row['after']} | {'; '.join(row.get('evidence', [])) or 'none'} | {row['verifier_status']} |" for row in rule_progress_rows) or "| none | unknown | unknown | none | not_required |",
    "",
    "## Recipe Review",
    "",
    f"- Status: `{recipe_review['status']}`",
    f"- Summary: {recipe_review['summary']}",
    "",
    "## Patch Plan Compliance",
    "",
    f"- Status: `{patch_plan_compliance['status']}`",
    f"- Summary: {patch_plan_compliance['summary']}",
    "",
    "## Template Application Review",
    "",
    f"- Status: `{template_application_review['status']}`",
    f"- Summary: {template_application_review['summary']}",
    "",
    "## Stack Profile Review",
    "",
    f"- Status: `{stack_profile_review['status']}`",
    f"- Summary: {stack_profile_review['summary']}",
    "",
    "## Rule Recheck Review",
    "",
    f"- Status: `{rule_recheck_review['status']}`",
    f"- Summary: {rule_recheck_review['summary']}",
    "",
    "## Scaffold vs Implementation Honesty",
    "",
    f"- Status: `{scaffold_honesty['status']}`",
    f"- Summary: {scaffold_honesty['summary']}",
    "",
    "## Runtime Feature Review",
    "",
    f"- Status: `{runtime_feature_review['status']}`",
    f"- Summary: {runtime_feature_review['summary']}",
    "",
    "## Adapter Compliance",
    "",
    f"- Status: `{adapter_compliance['status']}`",
    f"- Summary: {adapter_compliance['summary']}",
    "",
    "## Stack Confidence",
    "",
    f"- Status: `{stack_profile_review['status']}`",
    f"- Summary: {stack_profile_review['summary']}",
    "",
    "## Generated Files",
    "",
    f"- Status: `{generated_files_review['status']}`",
    f"- Summary: {generated_files_review['summary']}",
    "",
    "## Runtime Claim Honesty",
    "",
    f"- Status: `{runtime_claim_honesty['status']}`",
    f"- Summary: {runtime_claim_honesty['summary']}",
    "",
    "## UI/UX Evidence",
    "",
    f"- Status: `{ui_ux_evidence['status']}`",
    f"- Summary: {ui_ux_evidence['summary']}",
    "",
    "## Playwright Evidence",
    "",
    f"- Status: `{playwright_evidence_review['status']}`",
    f"- Summary: {playwright_evidence_review['summary']}",
    "",
    "## Metadata Synchronization",
    "",
    f"- Status: `{metadata_synchronization['status']}`",
    f"- Summary: {metadata_synchronization['summary']}",
    "",
    "## Evidence Bundle Review",
    "",
    f"- Status: `{evidence_bundle_review['status']}`",
    f"- Summary: {evidence_bundle_review['summary']}",
    "",
    "## Screenshot Evidence",
    "",
    f"- Status: `{screenshot_evidence_review['status']}`",
    f"- Summary: {screenshot_evidence_review['summary']}",
    "",
    "## Accessibility Evidence",
    "",
    f"- Status: `{accessibility_evidence_review['status']}`",
    f"- Summary: {accessibility_evidence_review['summary']}",
    "",
    "## Tutorial/PDF/Report Evidence",
    "",
    f"- Status: `{tutorial_pdf_report_evidence['status']}`",
    f"- Summary: {tutorial_pdf_report_evidence['summary']}",
    "",
    "## AI/NLAI Simulation Evidence",
    "",
    f"- Status: `{ai_nlai_simulation_evidence['status']}`",
    f"- Summary: {ai_nlai_simulation_evidence['summary']}",
    "",
    "## Token Usage Evidence",
    "",
    f"- Status: `{token_usage_evidence_review['status']}`",
    f"- Summary: {token_usage_evidence_review['summary']}",
    "",
    "## Evidence Gaps",
    "",
    f"- Status: `{evidence_gaps['status']}`",
    f"- Summary: {evidence_gaps['summary']}",
    "",
    "## Rule Recheck",
    "",
    f"- Status: `{rule_recheck_review['status']}`",
    f"- Summary: {rule_recheck_review['summary']}",
    "",
    "## Remaining Constitutional Gaps",
    "",
    "\n".join(f"- `{row['rule_id']}` remains `{row['after']}`" for row in rule_progress_rows if row["after"] in {"fail", "partial", "unknown"}) or "- None.",
    "",
    "## Maturity Impact Verification",
    "",
    f"- Status: `{maturity_impact['status']}`",
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

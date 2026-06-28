#!/usr/bin/env bash
# scripts/autospec-mvp-status.sh — summarize Autospec Constitution MVP readiness.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-mvp-status.sh [--repo-root DIR]

Writes .autospec/reports/mvp-status.json and .autospec/reports/mvp-status.md.
EOF
}

die() {
    printf 'autospec-mvp-status: %s\n' "$*" >&2
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
import sys

root = os.path.realpath(sys.argv[1])
reports = os.path.join(root, ".autospec", "reports")
state = os.path.join(root, ".autospec", "state")
script_root = os.path.dirname(os.path.realpath(__file__)) if "__file__" in globals() else os.path.join(root, "scripts")


def exists(path):
    return os.path.exists(os.path.join(root, path))


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


rules = load(os.path.join(state, "rule-check-results.json"), load(os.path.join(reports, "rule-check-results.json"), {"results": []}))
published = load(os.path.join(state, "published-issues.json"), {"issues": []})
preflight = load(os.path.join(reports, "preflight.json"), {})
command_audit = load(os.path.join(reports, "command-audit.json"), {})
state_validation = load(os.path.join(reports, "state-validation.json"), {})
sensitive_audit = load(os.path.join(reports, "sensitive-output-audit.json"), {})
mvp_smoke = load(os.path.join(reports, "mvp-smoke.json"), {})
report_index = load(os.path.join(reports, "report-index.json"), {})
recovery_status = load(os.path.join(reports, "recovery-status.json"), {})
report_quality = load(os.path.join(reports, "report-quality.json"), {})
check_type_coverage = load(os.path.join(reports, "check-type-coverage.json"), {})
template_coverage = load(os.path.join(reports, "template-coverage.json"), {})
release_candidate_gate = load(os.path.join(reports, "release-candidate-gate.json"), {})
spec_coverage = load(os.path.join(reports, "spec-coverage.json"), {})
coverage_requirements = spec_coverage.get("requirements", []) if isinstance(spec_coverage.get("requirements"), list) else []
critical_missing = [r for r in coverage_requirements if r.get("priority") == "critical" and r.get("status") == "missing" and r.get("requirement_type") != "target_app_scaffold"]
high_partial = [r for r in coverage_requirements if r.get("priority") == "high" and r.get("status") == "partial"]
documented_only = [r for r in coverage_requirements if r.get("status") == "documented_only"]
scaffolded = [r for r in coverage_requirements if r.get("status") == "scaffolded"]
implemented = [r for r in coverage_requirements if r.get("status") == "implemented"]
signals = {
    "preflight": preflight.get("verdict", "unknown"),
    "command_audit": "pass" if command_audit.get("summary", {}).get("commands_total", 0) else "unknown",
    "state_validation": state_validation.get("status", "unknown"),
    "sensitive_output_audit": sensitive_audit.get("status", "unknown"),
    "mvp_smoke": mvp_smoke.get("verdict", "unknown"),
    "report_index": "pass" if report_index.get("reports") else "unknown",
    "recovery_status": "warn" if recovery_status.get("active_lock") else "pass" if recovery_status else "unknown",
    "report_quality": report_quality.get("status", "unknown"),
    "check_type_coverage": "pass" if check_type_coverage.get("matrix") else "unknown",
    "template_coverage": "pass" if template_coverage.get("categories") else "unknown",
    "release_candidate_gate": release_candidate_gate.get("verdict", "unknown"),
    "spec_coverage": "fail" if critical_missing else "warn" if high_partial or documented_only else "pass" if coverage_requirements else "unknown",
}
blocking_values = {"blocked", "fail", "RC_BLOCKED", "RC_NOT_READY"}
warning_values = {"warn", "pass_with_warnings", "needs_fixes", "unknown", "RC_READY_WITH_WARNINGS"}
if any(value in blocking_values for value in signals.values()):
    readiness = "MVP_BLOCKED"
elif any(value in warning_values for value in signals.values()):
    readiness = "MVP_READY_WITH_WARNINGS"
else:
    readiness = "MVP_READY"
status = {
    "schema": 1,
    "readiness": readiness,
    "release_readiness_signals": signals,
    "spec_coverage": {
        "total_requirements": len(coverage_requirements),
        "critical_missing_requirements": len(critical_missing),
        "critical_missing_requirement_ids": [r.get("id") for r in critical_missing],
        "high_priority_partial_requirements": len(high_partial),
        "high_priority_partial_requirement_ids": [r.get("id") for r in high_partial],
        "documented_only_requirements": len(documented_only),
        "scaffolded_target_app_requirements": len(scaffolded),
        "implemented_requirements": len(implemented),
        "mvp_readiness_impact": "blocks_mvp_ready" if critical_missing else "warnings_only" if high_partial or documented_only else "clear" if coverage_requirements else "unknown",
    },
    "engine_capabilities": {
        "policy_sources": exists(".autospec/policy-sources.lock.json") or exists(".autospec/state/policy-sources.json"),
        "digital_twin": exists(".autospec/state/digital-twin.json"),
        "issue_plan_v3": exists(".autospec/reports/issue-plan-v3.json"),
        "worker": exists("scripts/autospec-worker-v1.sh"),
        "verifier": exists("scripts/autospec-verify-worker-pr.sh"),
        "supervisor": exists("scripts/autospec-supervisor-cycle.sh"),
        "onboarding": exists("scripts/autospec-onboard-existing-repo.sh"),
        "bootstrap": exists("scripts/autospec-bootstrap-new-project.sh"),
        "ai_nlai_scaffolds": exists("scripts/autospec-generate-ai-nlai-scaffold.sh"),
    },
    "policy_source_status": "present" if exists(".autospec/state/policy-sources.json") else "unknown",
    "baseline_source_status": "present" if exists(".autospec/reports/baseline-composition.json") else "unknown",
    "digital_twin_status": "present" if exists(".autospec/state/digital-twin.json") else "missing",
    "structured_rule_audit_status": "present" if rules.get("results") else "missing",
    "backlog_publishing_status": f"{sum(1 for item in published.get('issues', []) if item.get('plan_version') == 'v3')} v3 issues published",
    "known_limitations": "docs/KNOWN_LIMITATIONS.md",
    "recommended_next_milestones": [
        "Dogfood existing-repo onboarding on a real target repository.",
        "Run one dry-run structured-rule supervisor cycle.",
        "Expand structured Constitution coverage as policy repos mature.",
    ],
}
write_json(os.path.join(reports, "mvp-status.json"), status)
md = "\n".join([
    "# Autospec Constitution MVP Status",
    "",
    "## Engine capabilities",
    "",
    "\n".join(f"- {key}: `{str(value).lower()}`" for key, value in status["engine_capabilities"].items()),
    "",
    "## Policy source status",
    "",
    status["policy_source_status"],
    "",
    "## Baseline source status",
    "",
    status["baseline_source_status"],
    "",
    "## Digital Twin status",
    "",
    status["digital_twin_status"],
    "",
    "## Structured rule audit status",
    "",
    status["structured_rule_audit_status"],
    "",
    "## Backlog publishing status",
    "",
    status["backlog_publishing_status"],
    "",
    "## Worker/verifier/supervisor status",
    "",
    "- Worker, verifier, promotion, supervisor, loop, status, and guidance commands are local/operator invoked.",
    "",
    "## Onboarding support",
    "",
    "- Existing repository onboarding command is available.",
    "",
    "## Bootstrap support",
    "",
    "- New project bootstrap command is available.",
    "",
    "## AI/NLAI scaffold support",
    "",
    "- AI/NLAI scaffold generator is available.",
    "",
    "## Safety guarantees",
    "",
    "- No GitHub Actions or schedulers.",
    "- Dry-run first.",
    "- Confirm required for GitHub writes.",
    "- No auto-merge or self-approval.",
    "",
    "## Known limitations",
    "",
    "- See `docs/KNOWN_LIMITATIONS.md`.",
    "",
    "## Recommended next milestones",
    "",
    "\n".join(f"- {item}" for item in status["recommended_next_milestones"]),
    "",
    "## Release readiness signals",
    "",
    f"- Readiness: **{readiness}**",
    "\n".join(f"- {key}: `{value}`" for key, value in signals.items()),
    "",
    "## Spec coverage",
    "",
    f"- Total requirements: {len(coverage_requirements)}",
    f"- Critical missing requirements: {len(critical_missing)}",
    f"- High-priority partial requirements: {len(high_partial)}",
    f"- Documented-only requirements: {len(documented_only)}",
    f"- Scaffolded target-app requirements: {len(scaffolded)}",
    f"- Implemented requirements: {len(implemented)}",
    f"- MVP readiness impact: `{status['spec_coverage']['mvp_readiness_impact']}`",
])
write_text(os.path.join(reports, "mvp-status.md"), md)
print("mvp status: wrote reports")
PY

#!/usr/bin/env bash
# scripts/autospec-spec-coverage.sh — close original Autospec Constitution vision against implementation evidence.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-spec-coverage.sh [--repo-root DIR] [--dry-run|--confirm]

Dry-run is default. Confirm currently writes the same local reports/backlog as
dry-run; it never publishes issues or calls GitHub.
EOF
}

die() { printf 'autospec-spec-coverage: %s\n' "$*" >&2; exit 2; }

REPO_ROOT="$(pwd)"
CONFIRM=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$CONFIRM" <<'PY'
import json
import os
import re
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
confirm = sys.argv[2] == "1"
autospec = root / ".autospec"
state = autospec / "state"
reports = autospec / "reports"
backlog = autospec / "backlog" / "spec-coverage"
specs = root / "docs" / "specs"
for path in [state, reports, backlog, specs]:
    path.mkdir(parents=True, exist_ok=True)


def load_json(path: Path, default):
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        return data if isinstance(data, dict) else default
    except Exception:
        return default


def write_json(path: Path, data: dict):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, text: str):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text.rstrip() + "\n", encoding="utf-8")


def exists(rel: str) -> bool:
    return (root / rel).exists()


def evidence(paths):
    return [p for p in paths if exists(p)]


def slug(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-") or "requirement"


rules = load_json(state / "effective-rules.json", {"rules": []}).get("rules", [])
rule_results = load_json(state / "rule-check-results.json", load_json(reports / "rule-check-results.json", {"results": []})).get("results", [])
capabilities = load_json(state / "capability-registry.json", {"capabilities": []}).get("capabilities", [])
rule_ids = {r.get("rule_id") for r in rules + rule_results if isinstance(r, dict)}
cap_ids = {c.get("id") for c in capabilities if isinstance(c, dict)}

REQS = [
    ("autonomy.supervisor.single_cycle", "Single-cycle autonomous supervisor", "autonomous_development", "engine", ["scripts/autospec-supervisor-cycle.sh"], "implemented", "critical", "medium"),
    ("autonomy.loop.local_bounded", "Local bounded multi-cycle loop", "autonomous_development", "engine", ["scripts/autospec-supervisor-loop.sh"], "implemented", "critical", "medium"),
    ("autonomy.worker.low_risk_code", "One-issue low-risk worker", "autonomous_development", "worker", ["scripts/autospec-worker-v1.sh"], "implemented", "critical", "medium"),
    ("autonomy_v2.worker_capabilities", "Worker capability registry for bounded Autonomy v2", "autonomous_development", "worker", ["scripts/autospec-recipe-index.sh", ".autospec/state/worker-capabilities.yml"], "implemented", "critical", "medium"),
    ("autonomy_v2.recipe_registry", "Implementation recipe registry", "autonomous_development", "worker", ["scripts/autospec-recipe-index.sh", ".autospec/state/implementation-recipes.json"], "implemented", "critical", "medium"),
    ("autonomy_v2.rule_to_recipe", "Rule-to-recipe planning", "autonomous_development", "worker", ["scripts/autospec-rule-to-recipe-plan.sh"], "implemented", "critical", "medium"),
    ("autonomy_v2.patch_plan", "Patch plan contract before recipe execution", "autonomous_development", "worker", ["scripts/autospec-build-patch-plan.sh"], "implemented", "critical", "medium"),
    ("autonomy_v2.template_apply", "Safe dry-run-first template application engine", "autonomous_development", "worker", ["scripts/autospec-apply-template.sh"], "implemented", "high", "medium"),
    ("autonomy_v2.stack_profiles", "Stack profile detection for scaffold safety", "autonomous_development", "worker", ["scripts/autospec-detect-stack-profile.sh"], "implemented", "high", "medium"),
    ("autonomy_v2.recipe_execution", "Recipe-backed worker execution", "autonomous_development", "worker", ["scripts/autospec-worker-one.sh"], "implemented", "critical", "medium"),
    ("autonomy_v2.rule_recheck", "Rule-aware recheck after recipe execution", "autonomous_development", "verifier", ["scripts/autospec-rule-recheck.sh"], "implemented", "high", "medium"),
    ("autonomy_v2.scaffold_honesty", "Scaffold vs runtime implementation distinction", "autonomous_development", "verifier", ["docs/runbooks/SCAFFOLD_VS_IMPLEMENTATION.md"], "scaffolded", "critical", "medium"),
    ("runtime.adapters", "Target runtime adapter interface and index", "product_baseline", "engine", ["scripts/autospec-runtime-adapter-index.sh", ".autospec/state/runtime-adapters.json"], "implemented", "critical", "medium"),
    ("runtime.feature_slices", "Target-app feature slice registry", "product_baseline", "engine", ["scripts/autospec-feature-slice-index.sh", ".autospec/state/feature-slices.json"], "implemented", "critical", "medium"),
    ("runtime.implementation_plan", "Runtime implementation planning", "product_baseline", "engine", ["scripts/autospec-runtime-implementation-plan.sh"], "implemented", "critical", "medium"),
    ("runtime.generator", "Bounded target-app runtime feature generator", "product_baseline", "worker", ["scripts/autospec-generate-runtime-feature.sh"], "implemented", "critical", "medium"),
    ("runtime.metadata_sync", "Runtime metadata synchronization", "digital_twin", "engine", ["scripts/autospec-sync-runtime-metadata.sh"], "implemented", "high", "medium"),
    ("runtime.verification", "Runtime feature verification", "product_baseline", "verifier", ["scripts/autospec-verify-runtime-feature.sh"], "implemented", "critical", "medium"),
    ("runtime.playwright_evidence_generation", "Playwright evidence generation for runtime shells", "testing", "validator", ["scripts/autospec-generate-playwright-evidence.sh"], "implemented", "high", "medium"),
    ("runtime.worker_v4", "Worker v4 runtime feature support", "autonomous_development", "worker", ["scripts/autospec-worker-one.sh"], "implemented", "critical", "medium"),
    ("runtime.status", "Runtime feature status dashboard", "product_baseline", "engine", ["scripts/autospec-runtime-feature-status.sh"], "implemented", "high", "low"),
    ("autonomy.verifier.independent", "Independent verifier", "autonomous_development", "verifier", ["scripts/autospec-verify-worker-pr.sh"], "implemented", "critical", "medium"),
    ("autonomy.promotion_gate", "Promotion gate without merge/approval", "autonomous_development", "engine", ["scripts/autospec-promote-pr.sh"], "implemented", "critical", "low"),
    ("autonomy.remediation_loop", "Verifier remediation planner", "autonomous_development", "engine", ["scripts/autospec-plan-remediation.sh"], "implemented", "high", "low"),
    ("autonomy.stuck_guidance", "Stuck/guidance issue flow", "autonomous_development", "engine", ["scripts/autospec-publish-stuck.sh", "scripts/autospec-sync-guidance.sh"], "implemented", "high", "low"),
    ("autonomy.guide_skill", "autospec-guide operator skill", "autonomous_development", "documentation", ["skills/autospec-guide/SKILL.md"], "implemented", "medium", "low"),
    ("autonomy.locks_budgets_stop", "Locks, budgets, stop/resume", "autonomous_development", "engine", ["scripts/autospec-repo-lock.sh", "scripts/autospec-autonomy-budget.sh", "scripts/autospec-stop.sh", "scripts/autospec-resume.sh"], "implemented", "critical", "medium"),
    ("autonomy.issue_publish_sync", "GitHub issue publish/sync", "autonomous_development", "engine", ["scripts/autospec-publish-issues.sh", "scripts/autospec-sync-published-issues.sh"], "implemented", "critical", "medium"),
    ("autonomy.no_self_approval", "No auto-merge or self-approval safety", "autonomous_development", "validator", ["docs/RELEASE_READINESS.md", "docs/KNOWN_LIMITATIONS.md"], "documented_only", "critical", "low"),
    ("policy.structured_sources", "Structured Constitution and Baseline sources", "policy", "policy", ["scripts/autospec-load-policy-sources.sh", "scripts/autospec-validate-policy-sources.sh"], "implemented", "critical", "medium"),
    ("policy.structured_rules", "Structured rules, packs, and profile composition", "policy", "policy", ["scripts/autospec-extract-constitution-rules.sh", "scripts/autospec-baseline-compose.sh"], "implemented", "critical", "medium"),
    ("policy.lockfile", "Policy source lockfile", "policy", "policy", ["scripts/autospec-lock-policy-sources.sh"], "implemented", "high", "low"),
    ("policy.rule_checks", "Rule checks and quality gates", "policy", "validator", ["scripts/autospec-check-rules.sh"], "implemented", "critical", "medium"),
    ("policy.maturity_waivers_compatibility", "Maturity scoring, waivers, compatibility", "policy", "validator", ["scripts/autospec-constitutional-gap-v1.sh", "scripts/autospec-policy-compatibility.sh"], "implemented", "high", "medium"),
    ("digital_twin.inventory", "Repository inventory and technology registry", "digital_twin", "engine", ["scripts/autospec-build-digital-twin.sh"], "implemented", "critical", "medium"),
    ("digital_twin.surfaces", "API/UI/data/settings/permission/AI/MCP surfaces", "digital_twin", "engine", [".autospec/state/api-surface.json", ".autospec/state/ui-surface.json", ".autospec/state/ai-capabilities.json"], "partial", "high", "medium"),
    ("digital_twin.knowledge_graph", "Knowledge graph and digital twin summary", "digital_twin", "engine", [".autospec/state/knowledge-graph.json", ".autospec/state/digital-twin.json"], "partial", "high", "medium"),
    ("digital_twin.impact_drift", "Impact analysis and metadata drift", "digital_twin", "engine", ["scripts/autospec-impact-analysis.sh", "scripts/autospec-metadata-drift.sh"], "implemented", "high", "low"),
    ("onboarding.existing_repo", "Existing repo onboarding", "digital_twin", "engine", ["scripts/autospec-onboard-existing-repo.sh"], "implemented", "critical", "low"),
    ("onboarding.new_project", "New project bootstrap", "digital_twin", "engine", ["scripts/autospec-bootstrap-new-project.sh"], "implemented", "critical", "low"),
    ("engineering.design_patterns_adrs", "Design patterns, architecture notes, ADR expectations", "engineering", "validator", [], "validated", "medium", "low"),
    ("engineering.architecture_governance_audit", "Architecture governance audit and pattern guidance", "engineering", "engine", ["scripts/autospec-architecture-governance.sh", ".autospec/reports/architecture-governance.json"], "implemented", "high", "low"),
    ("engineering.library_standardization", "Library standardization and dependency sprawl detection", "engineering", "validator", ["scripts/autospec-check-rules.sh"], "validated", "high", "medium"),
    ("engineering.dependency_governance_audit", "Dependency governance audit", "engineering", "engine", ["scripts/autospec-dependency-governance.sh", ".autospec/reports/dependency-governance.json"], "implemented", "high", "medium"),
    ("engineering.modernization_migration", "Modernization planning and migration discipline", "engineering", "policy", [], "validated", "medium", "high"),
    ("engineering.modernization_planner", "Modernization planner", "engineering", "engine", ["scripts/autospec-modernization-plan.sh", ".autospec/reports/modernization-plan.json"], "implemented", "high", "medium"),
    ("engineering.risk_budgets", "Risk classification and patch budgets", "engineering", "worker", ["scripts/autospec-worker-v1.sh"], "implemented", "critical", "medium"),
    ("testing.unit_integration_contract", "Unit/integration/contract test doctrine", "testing", "validator", [], "validated", "high", "medium"),
    ("testing.playwright_viewport_visual", "Playwright viewport, screenshots, visual diffs", "testing", "validator", [], "validated", "high", "medium"),
    ("testing.playwright_evidence_audit", "Playwright evidence audit", "testing", "engine", ["scripts/autospec-playwright-evidence-audit.sh", ".autospec/reports/playwright-evidence-audit.json"], "implemented", "high", "low"),
    ("testing.validation_evidence", "Focused validation planning and evidence", "testing", "worker", ["scripts/autospec-worker-v1.sh", "scripts/autospec-verify-worker-pr.sh"], "implemented", "critical", "medium"),
    ("testing.performance_migration", "Performance and migration test expectations", "testing", "policy", [], "validated", "medium", "medium"),
    ("ui_ux.responsive_accessible_states", "Responsive UI, accessibility, empty/loading/error states", "ui_ux", "target_app_scaffold", [".autospec/templates/product-baseline/visual-design-system-spec.md"], "scaffolded", "high", "medium"),
    ("ui_ux.audit", "UI/UX quality audit", "ui_ux", "engine", ["scripts/autospec-ui-ux-audit.sh", ".autospec/reports/ui-ux-audit.json"], "implemented", "high", "low"),
    ("ui_ux.pretty_output", "Pretty output and raw JSON avoidance", "ui_ux", "target_app_scaffold", [".autospec/templates/ai-platform/pretty-rendering-spec.md"], "scaffolded", "high", "low"),
    ("docs.repo_in_app_rag", "Repo docs, in-app docs, RAG-ready docs", "docs_tutorial_pdf", "target_app_scaffold", [".autospec/templates/product-baseline/in-app-documentation-center-spec.md"], "scaffolded", "high", "low"),
    ("docs.artifact_audit", "Documentation artifact audit", "docs_tutorial_pdf", "engine", ["scripts/autospec-doc-artifact-audit.sh", ".autospec/reports/doc-artifact-audit.json"], "implemented", "high", "low"),
    ("docs.tutorials_screenshots", "Tutorials, screenshots, screencasts, narration/TTS spec", "docs_tutorial_pdf", "target_app_scaffold", [".autospec/templates/product-baseline/onboarding-tutorials-spec.md"], "scaffolded", "medium", "low"),
    ("docs.pdf_guides", "PDF guides and report formatting", "docs_tutorial_pdf", "target_app_scaffold", [".autospec/templates/product-baseline/reporting-dashboard-spec.md"], "scaffolded", "medium", "low"),
    ("docs.drift_detection", "Documentation drift detection", "docs_tutorial_pdf", "validator", ["scripts/autospec-metadata-drift.sh"], "partial", "medium", "low"),
    ("reporting.metrics", "Meaningful metrics tied to product purpose", "reporting_analytics_visualization", "target_app_scaffold", [".autospec/templates/product-baseline/analytics-metrics-spec.md"], "scaffolded", "high", "low"),
    ("reporting.analytics_audit", "Reporting and analytics audit", "reporting_analytics_visualization", "engine", ["scripts/autospec-reporting-analytics-audit.sh", ".autospec/reports/reporting-analytics-audit.json"], "implemented", "high", "low"),
    ("reporting.exports", "Report generation with CSV/PDF exports", "reporting_analytics_visualization", "target_app_scaffold", [".autospec/templates/product-baseline/reporting-dashboard-spec.md"], "scaffolded", "high", "low"),
    ("reporting.visualization_standard", "Charts by data type and chart library standardization", "reporting_analytics_visualization", "validator", [], "validated", "high", "medium"),
    ("ai.provider_abstraction", "AI provider abstraction", "ai_platform", "target_app_scaffold", [".autospec/templates/ai-platform/ai-platform-spec.md"], "scaffolded", "high", "medium"),
    ("ai.platform_audit", "AI platform audit", "ai_platform", "engine", ["scripts/autospec-ai-platform-audit.sh", ".autospec/reports/ai-platform-audit.json"], "implemented", "high", "low"),
    ("ai.openai_ollama_support", "OpenAI-compatible and Ollama/local support", "ai_platform", "target_app_scaffold", [".autospec/templates/ai-platform/ai-platform-spec.md"], "scaffolded", "high", "medium"),
    ("ai.settings_admin", "AI settings/admin pages", "ai_platform", "target_app_scaffold", [".autospec/templates/ai-platform/ai-settings-page-spec.md"], "scaffolded", "high", "medium"),
    ("ai.rag_embeddings", "RAG and embedding configuration", "ai_platform", "target_app_scaffold", [".autospec/templates/ai-platform/rag-assistant-spec.md"], "scaffolded", "high", "medium"),
    ("ai.agent_tool_memory_mcp", "Agent registry, tool registry, memory model, MCP registry", "ai_platform", "target_app_scaffold", [".autospec/templates/ai-platform/mcp-diagnostics-spec.md"], "scaffolded", "high", "medium"),
    ("ai.token_usage.multi_user_tracking", "Track token usage per user in multi-user apps", "ai_platform", "target_app_scaffold", [".autospec/templates/ai-platform/token-usage-dashboard-spec.md"], "scaffolded", "high", "medium"),
    ("ai.cost_quota_dashboard", "Cost tracking, quotas, budgets, usage dashboards", "ai_platform", "target_app_scaffold", [".autospec/templates/ai-platform/token-usage-dashboard-spec.md"], "scaffolded", "high", "medium"),
    ("nlai.capability_interface", "Natural-language application interface and capability tools", "nlai", "target_app_scaffold", [".autospec/templates/ai-platform/nlai-capability-interface-spec.md"], "scaffolded", "high", "medium"),
    ("nlai.audit", "NLAI audit", "nlai", "engine", ["scripts/autospec-nlai-audit.sh", ".autospec/reports/nlai-audit.json"], "implemented", "high", "low"),
    ("nlai.data_sql_file_reports", "Data querying, SQL explanation/visualization, file operations, reports", "nlai", "target_app_scaffold", [".autospec/templates/ai-platform/nlai-capability-interface-spec.md"], "scaffolded", "high", "medium"),
    ("nlai.pretty_rendering", "Pretty rendering of all outputs; avoid raw JSON", "nlai", "target_app_scaffold", [".autospec/templates/ai-platform/pretty-rendering-spec.md"], "scaffolded", "high", "low"),
    ("diagnostics.health_logs_metrics", "App health, logs, metrics, traces", "diagnostics", "target_app_scaffold", [".autospec/templates/product-baseline/diagnostics-status-page-spec.md"], "scaffolded", "high", "medium"),
    ("diagnostics.audit", "Diagnostics audit", "diagnostics", "engine", ["scripts/autospec-diagnostics-audit.sh", ".autospec/reports/diagnostics-audit.json"], "implemented", "high", "low"),
    ("diagnostics.white_screen_playwright", "Frontend white-screen diagnosis and Playwright repro capture", "diagnostics", "target_app_scaffold", [".autospec/templates/product-baseline/diagnostics-status-page-spec.md"], "scaffolded", "high", "medium"),
    ("diagnostics.incident_safe_remediation", "Incident reports and safe remediation boundaries", "diagnostics", "target_app_scaffold", [".autospec/templates/product-baseline/diagnostics-status-page-spec.md"], "scaffolded", "medium", "medium"),
    ("product.docs_settings_tutorials", "In-app docs, settings, AI settings, onboarding/tutorials", "product_baseline", "target_app_scaffold", [".autospec/templates/product-baseline/in-app-documentation-center-spec.md", ".autospec/templates/product-baseline/settings-area-spec.md"], "scaffolded", "high", "low"),
    ("product.feedback_status_search_admin", "Feedback/support, diagnostics/status, search/help, admin/operations", "product_baseline", "target_app_scaffold", [".autospec/templates/product-baseline/feedback-support-flow-spec.md", ".autospec/templates/product-baseline/diagnostics-status-page-spec.md"], "scaffolded", "medium", "low"),
    ("product.analytics_reporting", "Analytics/reporting product baseline", "product_baseline", "target_app_scaffold", [".autospec/templates/product-baseline/analytics-metrics-spec.md", ".autospec/templates/product-baseline/reporting-dashboard-spec.md"], "scaffolded", "high", "low"),
    ("doctrine.unified_audit", "Unified doctrine audit and local issue drafts", "policy", "engine", ["scripts/autospec-doctrine-audit.sh", ".autospec/reports/doctrine-audit.json", ".autospec/reports/doctrine-issue-plan.json"], "implemented", "high", "low"),
    ("release.check_type_coverage", "Check-type coverage matrix", "policy", "validator", ["scripts/autospec-check-type-coverage.sh", ".autospec/reports/check-type-coverage.json"], "implemented", "high", "low"),
    ("release.template_coverage", "Template and scaffold coverage matrix", "product_baseline", "validator", ["scripts/autospec-template-coverage.sh", ".autospec/reports/template-coverage.json"], "implemented", "high", "low"),
    ("release.report_quality", "Report quality gate", "policy", "validator", ["scripts/autospec-report-quality.sh", ".autospec/reports/report-quality.json"], "implemented", "high", "low"),
    ("release.rc_gate", "Release candidate gate", "policy", "engine", ["scripts/autospec-release-candidate-gate.sh", ".autospec/reports/release-candidate-gate.json"], "implemented", "critical", "low"),
    ("security.privacy_audit", "Security and privacy audit", "security", "engine", ["scripts/autospec-security-privacy-audit.sh", ".autospec/reports/security-privacy-audit.json"], "implemented", "high", "medium"),
    ("security.no_auto_auth_migrations", "No automatic auth/security/migration changes", "engineering", "documentation", ["docs/KNOWN_LIMITATIONS.md"], "deferred", "critical", "high"),
    ("automation.no_scheduled_background", "No scheduled/background automation", "autonomous_development", "documentation", ["docs/RELEASE_READINESS.md"], "deferred", "critical", "high"),
    ("target_app.full_ai_runtime", "Full AI/NLAI runtime implementation in arbitrary target repos", "ai_platform", "documentation", ["docs/KNOWN_LIMITATIONS.md"], "deferred", "high", "high"),
]


requirements = []
for rid, title, category, req_type, paths, default_status, priority, risk in REQS:
    ev = evidence(paths)
    status = default_status
    missing = []
    if paths and not ev:
        if default_status in {"implemented", "partial"}:
            status = "missing"
        missing = paths
    if req_type == "validator" and any(rid.split(".")[-1] in str(rule) for rule in rule_ids):
        status = "validated"
    requirements.append({
        "id": rid,
        "title": title,
        "category": category,
        "source": "spec",
        "source_files": ev or paths,
        "requirement_type": req_type,
        "expected_capability": title,
        "status": status,
        "evidence": ev,
        "missing_evidence": missing,
        "recommended_action": "No action required." if status in {"implemented", "scaffolded", "validated", "deferred"} else "Create or refine engine support.",
        "priority": priority,
        "risk": risk,
    })

requirements = sorted(requirements, key=lambda r: r["id"])
summary = {"total": len(requirements), "by_status": {}, "categories": {}}
for req in requirements:
    summary["by_status"][req["status"]] = summary["by_status"].get(req["status"], 0) + 1
    cat = summary["categories"].setdefault(req["category"], {"total": 0, "statuses": {}})
    cat["total"] += 1
    cat["statuses"][req["status"]] = cat["statuses"].get(req["status"], 0) + 1

write_json(state / "master-requirements.json", {"schema": 1, "mode": "confirm" if confirm else "dry_run", "requirements": requirements})
write_json(reports / "spec-coverage.json", {"schema": 1, "mode": "confirm" if confirm else "dry_run", "summary": summary, "requirements": requirements})

backlog_items = [r for r in requirements if r["status"] in {"missing", "partial", "documented_only"}]
for idx, req in enumerate(backlog_items, start=1):
    write_text(backlog / f"{idx:03d}-{slug(req['id'])}.md", "\n".join([
        f"# spec coverage: {req['title']}",
        "",
        f"Requirement ID: `{req['id']}`",
        "",
        "## Source files",
        "",
        "\n".join(f"- `{p}`" for p in req["source_files"]) or "- None.",
        "",
        f"## Current status\n\n{req['status']}",
        "",
        "## Missing evidence",
        "",
        "\n".join(f"- `{p}`" for p in req["missing_evidence"]) or "- Evidence exists but support is incomplete.",
        "",
        "## Implementation scope",
        "",
        req["recommended_action"],
        "",
        "## Acceptance criteria",
        "",
        f"- [ ] `{req['id']}` is reclassified from `{req['status']}` with concrete evidence.",
        "",
        "## Tests",
        "",
        "- Add or update focused unit/Bats tests for the command/report behavior.",
        "",
        "## Docs",
        "",
        "- Update relevant runbooks and known limitations.",
        "",
        f"## Risk\n\n{req['risk']}",
        "",
        "## Suggested labels",
        "",
        "- autospec:spec-coverage",
        f"- priority:{req['priority']}",
        "",
        "## Worker eligibility",
        "",
        "Worker v1/v2 may handle docs/spec/metadata-only or low-risk command/report changes. Human architecture guidance is required for high-risk runtime behavior.",
    ]))
write_json(reports / "spec-coverage-backlog.json", {"schema": 1, "issues": backlog_items})

matrix = "\n".join(
    f"| `{cat}` | {data['total']} | " + ", ".join(f"{k}: {v}" for k, v in sorted(data["statuses"].items())) + " |"
    for cat, data in sorted(summary["categories"].items())
)
req_rows = "\n".join(f"| `{r['id']}` | {r['category']} | {r['status']} | {r['priority']} | {', '.join(r['evidence']) or 'none'} |" for r in requirements)
md = "\n".join([
    "# Autospec Spec Coverage",
    "",
    "## Summary",
    "",
    f"- Total requirements: {summary['total']}",
    f"- Status counts: {json.dumps(summary['by_status'], sort_keys=True)}",
    "- runtime feature evidence is tracked through runtime adapters, feature slices, generation records, metadata sync, verification, worker v4, and runtime status reports.",
    "",
    "## Coverage Matrix",
    "",
    "| Category | Total | Statuses |",
    "| --- | ---: | --- |",
    matrix,
    "",
    "## Requirements",
    "",
    "| Requirement | Category | Status | Priority | Evidence |",
    "| --- | --- | --- | --- | --- |",
    req_rows,
    "",
    "## Required Follow-up Backlog",
    "",
    f"- Drafts written: {len(backlog_items)}",
])
write_text(reports / "spec-coverage.md", md)
write_text(reports / "spec-coverage-backlog.md", "# Spec Coverage Backlog\n\n" + "\n".join(f"- `{r['id']}`: {r['status']}" for r in backlog_items))
write_text(specs / "AUTOSPEC_CONSTITUTION_MASTER_SPEC_COVERAGE.md", md)
print("spec coverage: wrote reports")
PY

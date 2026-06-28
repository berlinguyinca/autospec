#!/usr/bin/env python3
"""Local doctrine audit helpers for Autospec.

All audits are read-only with respect to application code. They write only
Autospec state/reports/backlog artifacts and never call GitHub.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
from pathlib import Path


AUDIT_TITLES = {
    "architecture": "Architecture Governance",
    "ui-ux": "UI/UX Audit",
    "playwright": "Playwright Evidence Audit",
    "docs": "Documentation Artifact Audit",
    "reporting": "Reporting and Analytics Audit",
    "ai": "AI Platform Audit",
    "nlai": "NLAI Audit",
    "diagnostics": "Diagnostics Audit",
    "dependency": "Dependency Governance",
    "modernization": "Modernization Plan",
    "security": "Security and Privacy Audit",
}

REPORT_NAMES = {
    "architecture": "architecture-governance",
    "ui-ux": "ui-ux-audit",
    "playwright": "playwright-evidence-audit",
    "docs": "doc-artifact-audit",
    "reporting": "reporting-analytics-audit",
    "ai": "ai-platform-audit",
    "nlai": "nlai-audit",
    "diagnostics": "diagnostics-audit",
    "dependency": "dependency-governance",
    "modernization": "modernization-plan",
    "security": "security-privacy-audit",
}


def load_json(path: Path, default):
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        return data if isinstance(data, dict) else default
    except Exception:
        return default


def write_json(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text.rstrip() + "\n", encoding="utf-8")


def rel_files(root: Path) -> list[str]:
    files = []
    for base, dirs, names in os.walk(root):
        rel_base = Path(base).relative_to(root)
        if any(part in {".git", "node_modules", "__pycache__"} for part in rel_base.parts):
            dirs[:] = []
            continue
        for name in names:
            files.append((rel_base / name).as_posix())
    return sorted(files)


def read_all_text(root: Path, files: list[str]) -> str:
    chunks = []
    for rel in files:
        path = root / rel
        if path.stat().st_size > 300_000:
            continue
        try:
            chunks.append(rel)
            chunks.append(path.read_text(encoding="utf-8", errors="ignore"))
        except OSError:
            pass
    return "\n".join(chunks).lower()


def check(name: str, ok: bool, evidence: list[str] | None = None, missing: list[str] | None = None, partial: bool = False) -> dict:
    status = "pass" if ok else "partial" if partial else "fail"
    return {"status": status, "evidence": evidence or [], "missing_evidence": missing or [], "summary": name}


def keyword_check(text: str, keywords: list[str]) -> dict:
    hits = [k for k in keywords if k.lower() in text]
    return check(", ".join(keywords), len(hits) >= max(1, min(2, len(keywords))), hits, [k for k in keywords if k not in hits], bool(hits))


def package_text(root: Path) -> str:
    chunks = []
    for name in ["package.json", "pyproject.toml", "requirements.txt", "go.mod", "Cargo.toml"]:
        path = root / name
        if path.exists():
            chunks.append(path.read_text(encoding="utf-8", errors="ignore"))
    return "\n".join(chunks).lower()


def base_context(root: Path) -> tuple[list[str], str, str]:
    files = rel_files(root)
    return files, read_all_text(root, files), package_text(root)


def audit_architecture(root: Path, target_file: str = "") -> dict:
    files, text, _pkg = base_context(root)
    adr_files = [
        f for f in files
        if not f.startswith(".autospec/templates/")
        and re.search(r"(^|/)adr[s]?/|architecture.*\.md|decision", f, re.I)
    ]
    arch_map = (root / ".autospec/state/architecture-map.json").exists()
    pattern = "service/domain layer"
    low = target_file.lower()
    if any(w in low for w in ["ui", "component", "page", "form"]):
        pattern = "reducer/state-machine"
    elif "api" in low:
        pattern = "adapter/client"
    elif "ai" in low or "provider" in low:
        pattern = "provider adapter"
    elif "report" in low:
        pattern = "builder/template"
    return {
        "schema": 1,
        "category": "architecture",
        "checks": {
            "adrs": check("ADR documentation", bool(adr_files), adr_files, ["docs/adr/"]),
            "architecture_map": check("Architecture map", arch_map, [".autospec/state/architecture-map.json"] if arch_map else [], [".autospec/state/architecture-map.json"] if not arch_map else []),
            "design_pattern_rationale": keyword_check(text, ["design pattern", "architecture notes", "adr"]),
            "impact_analysis": check("Impact analysis command", (root / "scripts/autospec-impact-analysis.sh").exists(), ["scripts/autospec-impact-analysis.sh"] if (root / "scripts/autospec-impact-analysis.sh").exists() else [], ["scripts/autospec-impact-analysis.sh"]),
        },
        "pattern_guidance": {"target": target_file, "recommended_pattern": pattern},
    }


def audit_ui(root: Path) -> dict:
    files, text, _pkg = base_context(root)
    ui_files = [f for f in files if re.search(r"src/(components|pages|app)|\\.tsx$|\\.jsx$", f)]
    return {"schema": 1, "category": "ui-ux", "checks": {
        "ui_surface": check("UI surface", bool(ui_files), ui_files[:10], ["src/components"]),
        "design_tokens": keyword_check(text, ["token", "spacing", "color"]),
        "component_reuse": keyword_check(text, ["component", "props"]),
        "responsive_layout": keyword_check(text, ["viewport", "responsive", "media"]),
        "accessibility": keyword_check(text, ["accessibility", "aria", "a11y"]),
        "keyboard_focus": keyword_check(text, ["keyboard", "focus", "tab"]),
        "empty_loading_error_states": keyword_check(text, ["empty", "loading", "error"]),
        "visual_regression": keyword_check(text, ["visual", "screenshot"]),
        "pretty_rendering": keyword_check(text, ["pretty rendering", "render json", "viewer"]),
        "raw_json_avoidance": keyword_check(text, ["raw json", "avoid raw json"]),
    }}


def audit_playwright(root: Path) -> dict:
    files, text, pkg = base_context(root)
    config = [f for f in files if "playwright.config" in f]
    e2e = [f for f in files if "e2e" in f or "playwright" in f]
    viewports = ["320x640", "375x812", "768x1024", "1024x768", "1440x900", "1920x1080"]
    found_viewports = [v for v in viewports if v in text or v.replace("x", ", height: ") in text]
    return {"schema": 1, "category": "playwright", "checks": {
        "playwright_config": check("Playwright config", bool(config) or "@playwright/test" in pkg, config or ["package.json"], ["playwright.config.ts"]),
        "e2e_tests": check("E2E tests", bool(e2e), e2e[:10], ["tests/e2e"]),
        "viewport_matrix": check("Viewport matrix", len(found_viewports) == len(viewports), found_viewports, [v for v in viewports if v not in found_viewports], bool(found_viewports)),
        "screenshot_artifacts": keyword_check(text, ["screenshot"]),
        "visual_diff_artifacts": keyword_check(text, ["visual diff", "tohavescreenshot", "snapshot"]),
        "accessibility_checks": keyword_check(text, ["axe", "accessibility", "aria"]),
        "tutorial_capture": keyword_check(text, ["tutorial", "capture"]),
        "white_screen_flow": keyword_check(text, ["white-screen", "white screen", "console"]),
        "keyboard_navigation": keyword_check(text, ["keyboard", "tab", "focus"]),
    }}


def audit_docs(root: Path) -> dict:
    files, text, _pkg = base_context(root)
    return {"schema": 1, "category": "documentation", "checks": {
        "readme": check("README", any(Path(f).name.lower().startswith("readme") for f in files), [f for f in files if Path(f).name.lower().startswith("readme")], ["README.md"]),
        "user_guide": keyword_check(text, ["user guide", "user docs"]),
        "api_cli_docs": keyword_check(text, ["api", "cli", "command"]),
        "in_app_docs": keyword_check(text, ["in-app docs", "documentation center"]),
        "rag_ready_docs": keyword_check(text, ["rag-ready", "citations", "source metadata"]),
        "tutorials": keyword_check(text, ["tutorial"]),
        "tutorial_screenshots": keyword_check(text, ["screenshot"]),
        "tutorial_pdfs": keyword_check(text, ["pdf"]),
        "screencast": keyword_check(text, ["screencast", "video"]),
        "tts_narration": keyword_check(text, ["tts", "narration"]),
        "troubleshooting": keyword_check(text, ["troubleshooting"]),
        "docs_drift": keyword_check(text, ["docs drift", "metadata drift"]),
    }}


def audit_reporting(root: Path) -> dict:
    files, text, pkg = base_context(root)
    chart_libs = [lib for lib in ["recharts", "chart.js", "d3", "echarts", "highcharts", "plotly"] if lib in pkg]
    return {"schema": 1, "category": "reporting", "checks": {
        "metrics_definitions": keyword_check(text, ["metric", "metrics definition"]),
        "dashboard_reports": keyword_check(text, ["dashboard", "report"]),
        "report_generation": keyword_check(text, ["report generation", "pdf", "csv"]),
        "csv_pdf_exports": keyword_check(text, ["csv", "pdf", "export"]),
        "chart_library_standardization": check("Chart library standardization", len(chart_libs) <= 1, chart_libs, ["single chart library"], len(chart_libs) > 1),
        "chart_selection": keyword_check(text, ["chart selection", "visualization"]),
        "statistics_product_purpose": keyword_check(text, ["statistics", "product purpose"]),
        "report_templates": keyword_check(text, ["report template"]),
    }, "chart_libraries": chart_libs}


def audit_ai(root: Path) -> dict:
    _files, text, pkg = base_context(root)
    return {"schema": 1, "category": "ai", "checks": {
        "provider_abstraction": keyword_check(text, ["provider abstraction", "provider adapter"]),
        "openai_compatible": keyword_check(text + pkg, ["openai", "base url"]),
        "ollama": keyword_check(text + pkg, ["ollama"]),
        "model_settings": keyword_check(text, ["model settings", "model selection"]),
        "ai_settings_admin": keyword_check(text, ["ai settings", "admin"]),
        "rag_config": keyword_check(text, ["rag", "retrieval"]),
        "embedding_config": keyword_check(text, ["embedding"]),
        "agent_registry": keyword_check(text, ["agent registry"]),
        "tool_registry": keyword_check(text, ["tool registry"]),
        "memory_model": keyword_check(text, ["memory model"]),
        "mcp_registry": keyword_check(text, ["mcp registry", "mcp"]),
        "token_usage": keyword_check(text, ["token usage", "tokens"]),
        "cost_tracking": keyword_check(text, ["cost tracking", "cost"]),
        "usage_dashboard": keyword_check(text, ["usage dashboard"]),
        "budget_quota": keyword_check(text, ["budget", "quota"]),
        "audit_logging": keyword_check(text, ["audit logging", "audit log"]),
        "citations": keyword_check(text, ["citations", "sources"]),
    }}


def audit_nlai(root: Path) -> dict:
    _files, text, _pkg = base_context(root)
    return {"schema": 1, "category": "nlai", "checks": {
        "capability_registry": check("Capability registry", (root / ".autospec/state/capability-registry.json").exists(), [".autospec/state/capability-registry.json"], [".autospec/state/capability-registry.json"]),
        "tool_interface": keyword_check(text, ["tool interface", "capability interface"]),
        "data_querying": keyword_check(text, ["data query", "query data"]),
        "sql_generation": keyword_check(text, ["sql generation", "sql safety"]),
        "visualization": keyword_check(text, ["visualization"]),
        "file_operations": keyword_check(text, ["file operations", "file preview"]),
        "report_generation": keyword_check(text, ["report generation"]),
        "workflow_execution": keyword_check(text, ["workflow execution"]),
        "pretty_rendering": keyword_check(text, ["pretty rendering", "viewer"]),
        "permission_checks": keyword_check(text, ["permission", "rbac"]),
        "explainability": keyword_check(text, ["explain", "citations"]),
    }}


def audit_diagnostics(root: Path) -> dict:
    _files, text, _pkg = base_context(root)
    return {"schema": 1, "category": "diagnostics", "checks": {
        "health_status_page": keyword_check(text, ["health", "status page"]),
        "health_endpoint": keyword_check(text, ["health endpoint", "/health"]),
        "logs": keyword_check(text, ["logs", "logging"]),
        "metrics": keyword_check(text, ["metrics"]),
        "tracing": keyword_check(text, ["trace", "tracing"]),
        "console_capture": keyword_check(text, ["console capture", "console"]),
        "network_capture": keyword_check(text, ["network capture", "network"]),
        "white_screen": keyword_check(text, ["white-screen", "white screen"]),
        "playwright_repro": keyword_check(text, ["playwright repro", "playwright"]),
        "incident_report": keyword_check(text, ["incident report"]),
        "mcp_diagnostics": keyword_check(text, ["mcp diagnostics", "mcp registry"]),
        "safe_remediation": keyword_check(text, ["safe remediation", "stuck/guidance"]),
    }}


def audit_dependency(root: Path) -> dict:
    files, text, pkg = base_context(root)
    manifests = [f for f in files if Path(f).name in {"package.json", "pyproject.toml", "requirements.txt", "go.mod", "Cargo.toml"}]
    lockfiles = [f for f in files if Path(f).name in {"package-lock.json", "pnpm-lock.yaml", "yarn.lock", "poetry.lock", "go.sum", "Cargo.lock"}]
    chart_libs = [lib for lib in ["recharts", "chart.js", "d3", "echarts", "plotly"] if lib in pkg]
    test_runners = [lib for lib in ["vitest", "jest", "mocha", "pytest"] if lib in pkg]
    return {"schema": 1, "category": "dependency", "checks": {
        "package_manifests": check("Package manifests", bool(manifests), manifests, ["package manifest"]),
        "lockfiles": check("Lockfiles", bool(lockfiles), lockfiles, ["lockfile"]),
        "charting_library_sprawl": check("Chart library sprawl", len(chart_libs) <= 1, chart_libs, ["single chart library"], len(chart_libs) > 1),
        "test_runner_sprawl": check("Test runner sprawl", len(test_runners) <= 1, test_runners, ["single test runner or ADR"], len(test_runners) > 1),
        "modernization_plan": keyword_check(text, ["modernization plan"]),
        "migration_docs": keyword_check(text, ["migration plan"]),
        "dependency_update_policy": keyword_check(text, ["dependency update"]),
    }, "side_effects": {"dependencies_updated": False}}


def audit_security(root: Path) -> dict:
    _files, text, _pkg = base_context(root)
    return {"schema": 1, "category": "security", "checks": {
        "threat_model": keyword_check(text, ["threat model"]),
        "secret_reference_policy": keyword_check(text, ["secret reference", "secrets"]),
        "permission_model": keyword_check(text, ["permission model", "rbac", "abac"]),
        "audit_logs": keyword_check(text, ["audit log"]),
        "pii_inventory": keyword_check(text, ["pii inventory", "personal data"]),
        "retention_deletion": keyword_check(text, ["retention", "deletion"]),
        "privacy_docs": keyword_check(text, ["privacy"]),
        "security_review": keyword_check(text, ["security review"]),
    }}


AUDIT_FUNCS = {
    "architecture": audit_architecture,
    "ui-ux": audit_ui,
    "playwright": audit_playwright,
    "docs": audit_docs,
    "reporting": audit_reporting,
    "ai": audit_ai,
    "nlai": audit_nlai,
    "diagnostics": audit_diagnostics,
    "dependency": audit_dependency,
    "security": audit_security,
}


def findings_from_report(report: dict) -> list[dict]:
    findings = []
    for name, item in report.get("checks", {}).items():
        if item.get("status") in {"fail", "partial"}:
            findings.append({"check": name, "status": item.get("status"), "summary": item.get("summary", name), "missing_evidence": item.get("missing_evidence", [])})
    return findings


def write_audit(root: Path, audit: str, report: dict) -> None:
    reports = root / ".autospec/reports"
    state = root / ".autospec/state"
    reports.mkdir(parents=True, exist_ok=True)
    state.mkdir(parents=True, exist_ok=True)
    name = REPORT_NAMES[audit]
    findings = findings_from_report(report)
    report = {**report, "findings": findings, "side_effects": {"github_writes": False, "dependencies_updated": False, **report.get("side_effects", {})}}
    write_json(reports / f"{name}.json", report)
    write_json(state / f"{name}.json", report)
    rows = "\n".join(f"| `{key}` | {value.get('status')} | {', '.join(value.get('evidence', [])) or 'none'} | {', '.join(value.get('missing_evidence', [])) or 'none'} |" for key, value in sorted(report.get("checks", {}).items()))
    extra = ""
    if audit == "architecture":
        extra = "\n## Pattern guidance\n\n" + json.dumps(report.get("pattern_guidance", {}), sort_keys=True)
    if audit == "ui-ux":
        extra = "\n## UI/UX Notes\n\n- raw JSON avoidance is checked as a human-output quality gate."
    write_text(reports / f"{name}.md", "\n".join([
        f"# Autospec {AUDIT_TITLES[audit]}",
        "",
        "## Summary",
        "",
        f"- Findings: {len(findings)}",
        "- GitHub writes: false",
        "",
        "## Checks",
        "",
        "| Check | Status | Evidence | Missing |",
        "| --- | --- | --- | --- |",
        rows,
        extra,
    ]))


def modernization(root: Path) -> None:
    report = audit_dependency(root)
    backlog = root / ".autospec/backlog/modernization"
    backlog.mkdir(parents=True, exist_ok=True)
    categories = ["patch", "minor", "major", "security", "runtime", "framework", "tooling", "library-consolidation"]
    for idx, cat in enumerate(categories, start=1):
        write_text(backlog / f"{idx:03d}-{cat}.md", f"# modernization: {cat}\n\nNo dependency update is performed by this plan.\n")
    report["modernization_categories"] = categories
    write_audit(root, "modernization", report)


def doctrine(root: Path, selected: str = "all") -> None:
    audits = ["architecture", "ui-ux", "playwright", "docs", "reporting", "ai", "nlai", "diagnostics", "dependency", "security"]
    aggregate = []
    for audit in audits:
        if selected not in {"all", audit}:
            continue
        report = AUDIT_FUNCS[audit](root)
        write_audit(root, audit, report)
        for finding in findings_from_report(report):
            aggregate.append({"category": audit, **finding})
    modernization(root)
    aggregate.extend({"category": "modernization", **f} for f in findings_from_report(audit_dependency(root)))
    reports = root / ".autospec/reports"
    backlog = root / ".autospec/backlog/doctrine"
    reports.mkdir(parents=True, exist_ok=True)
    backlog.mkdir(parents=True, exist_ok=True)
    for idx, finding in enumerate(aggregate, start=1):
        write_text(backlog / f"{idx:03d}-{finding['category']}-{finding['check']}.md", "\n".join([
            f"# doctrine: {finding['category']} {finding['check']}",
            "",
            f"Doctrine category: `{finding['category']}`",
            f"Finding: {finding['summary']}",
            f"Source rule/check: `{finding['check']}`",
            f"Evidence missing: {', '.join(finding.get('missing_evidence', [])) or 'none'}",
            "Scaffold/template available: `.autospec/templates/`",
            "",
            "## Implementation scope",
            "",
            "Engine-side audit/template/backlog support only unless the target repository explicitly scopes implementation.",
            "",
            "## Tests",
            "",
            "- Add or update focused tests for the target repo behavior or Autospec audit evidence.",
            "",
            "## Docs",
            "",
            "- Update the relevant runbook, spec, or in-app documentation source.",
            "",
            "## Acceptance criteria",
            "",
            f"- [ ] `{finding['check']}` has pass/partial evidence or an explicit deferral.",
            "",
            "## Risk",
            "",
            "low for docs/templates; medium/high if target runtime behavior is requested.",
            "",
            "## Worker eligibility",
            "",
            "Worker may handle docs/spec/template changes. Human guidance is required for target-app runtime/security/data behavior.",
        ]))
    scorecard = {}
    for finding in aggregate:
        row = scorecard.setdefault(finding["category"], {"findings": 0})
        row["findings"] += 1
    result = {"schema": 1, "scorecard": scorecard, "findings": aggregate, "side_effects": {"github_writes": False, "issues_published": False}}
    write_json(reports / "doctrine-audit.json", result)
    write_json(reports / "doctrine-issue-plan.json", {"schema": 1, "issues": aggregate})
    write_text(reports / "doctrine-issue-plan.md", "# Doctrine Issue Plan\n\n" + "\n".join(f"- `{f['category']}.{f['check']}` {f['status']}" for f in aggregate))
    write_text(reports / "doctrine-audit.md", "\n".join([
        "# Autospec Doctrine Audit",
        "",
        "## Executive summary",
        "",
        f"- Findings: {len(aggregate)}",
        "- GitHub writes: false",
        "",
        "## Category scorecard",
        "",
        "\n".join(f"- `{cat}`: {row['findings']} findings" for cat, row in sorted(scorecard.items())),
        "",
        "## Critical gaps",
        "",
        "\n".join(f"- `{f['category']}.{f['check']}`" for f in aggregate[:10]) or "- None.",
        "",
        "## High-value scaffolds available",
        "",
        "- See `.autospec/templates/`.",
        "",
        "## Target-app runtime gaps",
        "",
        "- Runtime AI/NLAI/security/data behavior remains target-repo scoped.",
        "",
        "## Engine gaps",
        "",
        "- See doctrine issue plan.",
        "",
        "## Suggested backlog",
        "",
        "- `.autospec/backlog/doctrine/`",
        "",
        "## Next commands",
        "",
        "- `bash scripts/autospec-spec-coverage.sh --dry-run`",
    ]))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--audit", required=True)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--confirm", action="store_true")
    parser.add_argument("--file", default="")
    parser.add_argument("--issue", default="")
    parser.add_argument("--category", default="all")
    parser.add_argument("--all", action="store_true")
    args = parser.parse_args()
    root = Path(args.repo_root).resolve()
    if args.audit == "modernization":
        modernization(root)
    elif args.audit == "doctrine":
        doctrine(root, args.category if args.category != "all" else "all")
    elif args.audit == "architecture":
        write_audit(root, args.audit, audit_architecture(root, args.file))
    else:
        write_audit(root, args.audit, AUDIT_FUNCS[args.audit](root))
    print(f"{args.audit}: wrote reports")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

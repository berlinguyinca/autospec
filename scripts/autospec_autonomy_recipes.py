#!/usr/bin/env python3
"""Built-in implementation recipes, their YAML files, and the recipe index.

Extracted from autospec-autonomy-v2-lib.py to bring that file under the repo's
file-size gate. The recipe builder was an inner function of built_in_recipes and
is now module level, so the recipe rows move verbatim and the two oversized
functions come under the per-function gate. Behaviour is unchanged.
"""

from __future__ import annotations

from pathlib import Path

from autospec_autonomy_capabilities import ensure_capabilities
from autospec_autonomy_io import (
    FORBIDDEN_PATHS,
    reports,
    state,
    write_json,
    write_text,
    yaml_scalar,
)

RECHECK_COMMAND = "bash scripts/autospec-rule-recheck.sh --dry-run"


def recipe(rid, title, category, capability, checks, mode="scaffold", expected=None, stacks=None, risk="low"):
    return {
        "id": rid,
        "title": title,
        "summary": title,
        "category": category,
        "capability": capability,
        "applies_to_rules": checks,
        "applies_to_capabilities": [capability],
        "repo_conditions": {"application_types": ["web", "cli"], "technologies": stacks or [], "required_files": [], "optional_files": []},
        "risk": {"level": risk, "requires_human_guidance": risk == "high", "requires_architecture_review": risk == "high"},
        "implementation": {"mode": mode, "allowed_paths": ["docs/**", ".autospec/**", "tests/**", "src/**"], "forbidden_paths": FORBIDDEN_PATHS, "max_files_changed": 5, "max_lines_changed": 250, "expected_files": {"create": expected or [], "update": [], "inspect": []}},
        "test_plan": {"required": mode in {"test", "scaffold"}, "suggested_commands": [RECHECK_COMMAND], "generated_tests": expected or []},
        "validation": {"commands": [RECHECK_COMMAND], "required_evidence": ["report"]},
        "docs": {"update": ["docs/specs"]},
        "metadata": {"update": [".autospec/state"]},
        "acceptance_criteria": [f"{title} scaffold/spec exists.", "Rule recheck is honest about before/after status."],
        "stuck_if": ["stack confidence is low", "capability is disabled", "forbidden path required"],
        "status": "supported",
    }


def built_in_recipes() -> list[dict]:
    return [
        recipe("in-app-docs-center", "In-app documentation center", "docs", "docs", ["required_in_app_documentation", "docs.in_app_docs"], "docs", ["docs/specs/in-app-documentation-center.md"]),
        recipe("rag-ready-docs", "RAG-ready documentation", "docs", "docs", ["required_rag_ready_documentation"], "docs", ["docs/specs/rag-ready-docs.md"]),
        recipe("tutorial-docs", "Tutorial documentation", "docs", "docs", ["required_tutorials_for_user_workflows"], "docs", ["docs/tutorials/autospec-tutorial.md"]),
        recipe("playwright-viewport-matrix", "Playwright viewport matrix", "tests", "playwright_scaffold", ["required_playwright_viewport_matrix", "testing.playwright.viewport"], "test", ["tests/e2e/autospec-viewport.spec.ts"], ["react", "vite", "nextjs", "playwright"]),
        recipe("accessibility-smoke", "Accessibility smoke test", "tests", "playwright_scaffold", ["required_accessibility_checklist", "required_playwright_accessibility_flow"], "test", ["tests/e2e/autospec-accessibility.spec.ts"], ["playwright"]),
        recipe("metadata-drift-test", "Metadata drift test", "tests", "tests", ["required_docs_drift_check"], "test", ["tests/unit/autospec-metadata-drift.bats"]),
        recipe("settings-page-scaffold", "Settings page scaffold", "ui", "settings_scaffold", ["required_settings_area", "required_ai_settings_page"], "scaffold", ["src/components/AutospecSettings.tsx"], ["react", "vite", "nextjs"], "medium"),
        recipe("documentation-route-scaffold", "Documentation route scaffold", "ui", "ui_scaffold", ["required_in_app_docs"], "scaffold", ["src/components/AutospecDocs.tsx"], ["react", "vite", "nextjs"], "medium"),
        recipe("status-page-scaffold", "Status page scaffold", "ui", "diagnostics_scaffold", ["required_status_page"], "scaffold", ["docs/specs/status-page.md"], ["react", "vite", "nextjs"]),
        recipe("feedback-flow-scaffold", "Feedback flow scaffold", "ui", "ui_scaffold", ["required_feedback_flow"], "scaffold", ["docs/specs/feedback-flow.md"], ["react", "vite", "nextjs"], "medium"),
        recipe("ai-provider-config-scaffold", "AI provider config scaffold", "ai", "ai_scaffold", ["required_ai_provider_abstraction", "ai.provider"], "scaffold", ["docs/specs/ai-provider-abstraction.md"]),
        recipe("rag-assistant-scaffold", "RAG assistant scaffold", "ai", "ai_scaffold", ["required_rag_indexing", "required_rag_capability"], "scaffold", ["docs/specs/rag-assistant.md"]),
        recipe("token-usage-scaffold", "Token usage scaffold", "ai", "ai_scaffold", ["required_token_usage_tracking", "required_multi_user_token_usage_tracking"], "planning_only", ["docs/specs/token-usage.md"]),
        recipe("ai-settings-page-scaffold", "AI settings page scaffold", "ai", "ai_scaffold", ["required_ai_settings_page", "required_ai_settings"], "scaffold", ["docs/specs/ai-settings-page.md"]),
        recipe("mcp-registry-scaffold", "MCP registry scaffold", "ai", "ai_scaffold", ["required_mcp_registry", "required_diagnostic_mcp_registry"], "scaffold", ["docs/specs/mcp-registry.md"]),
        recipe("capability-interface-scaffold", "NLAI capability interface", "nlai", "nlai_scaffold", ["required_nlai_capability_registry", "required_nlai_tool_interface"], "scaffold", ["docs/specs/nlai-capability-interface.md"]),
        recipe("pretty-rendering-scaffold", "Pretty rendering scaffold", "nlai", "nlai_scaffold", ["required_pretty_human_output", "required_pretty_rendering"], "scaffold", ["docs/specs/pretty-rendering.md"]),
        recipe("sql-query-interface-scaffold", "SQL query interface spec", "nlai", "nlai_scaffold", ["required_nlai_sql_generation_for_data_apps"], "planning_only", ["docs/specs/sql-query-interface.md"]),
        recipe("file-operations-interface-scaffold", "File operations interface spec", "nlai", "nlai_scaffold", ["required_nlai_file_operations_for_file_apps"], "planning_only", ["docs/specs/file-operations-interface.md"]),
        recipe("health-check-scaffold", "Health check scaffold", "diagnostics", "diagnostics_scaffold", ["required_health_check"], "scaffold", ["docs/specs/health-check.md"]),
        recipe("white-screen-diagnostic-scaffold", "White-screen diagnostic scaffold", "diagnostics", "diagnostics_scaffold", ["required_white_screen_diagnosis"], "test", ["tests/e2e/autospec-white-screen.spec.ts"], ["playwright"]),
        recipe("incident-report-scaffold", "Incident report scaffold", "diagnostics", "diagnostics_scaffold", ["required_incident_report_template"], "docs", ["docs/specs/incident-report-template.md"]),
        recipe("metrics-definition-scaffold", "Metrics definition scaffold", "reporting", "reporting_scaffold", ["required_metrics_definitions"], "docs", ["docs/specs/metrics-definition.yml"]),
        recipe("report-template-scaffold", "Report template scaffold", "reporting", "reporting_scaffold", ["required_report_generation"], "docs", ["docs/specs/report-template.md"]),
        recipe("visualization-standard-scaffold", "Visualization standard scaffold", "reporting", "reporting_scaffold", ["required_visualization_library_standard"], "docs", ["docs/specs/visualization-standard.md"]),
        recipe("governance-policy", "Dependency governance policy", "dependencies", "dependency_planning", ["required_dependency_governance_policy"], "planning_only", ["docs/specs/dependency-governance.md"]),
        recipe("modernization-plan", "Modernization plan", "dependencies", "modernization_planning", ["required_modernization_plan"], "planning_only", ["docs/specs/modernization-plan.md"]),
        recipe("threat-model-doc", "Threat model doc", "security-privacy", "security_privacy_docs", ["required_threat_model"], "docs", ["docs/specs/threat-model.md"], risk="medium"),
        recipe("permission-model-doc", "Permission model doc", "security-privacy", "security_privacy_docs", ["required_permission_model_for_multi_user"], "docs", ["docs/specs/permission-model.md"], risk="medium"),
        recipe("pii-retention-doc", "PII retention doc", "security-privacy", "security_privacy_docs", ["required_pii_inventory", "required_retention_policy"], "docs", ["docs/specs/pii-retention.md"], risk="medium"),
    ]


def list_block(name: str, values: list[str], indent: str = "") -> list[str]:
    return [f"{indent}{name}:"] + ([f"{indent}  - {yaml_scalar(value)}" for value in values] if values else [f"{indent}  []"])


def recipe_yaml_lines(entry: dict) -> list[str]:
    impl = entry["implementation"]
    expected = impl["expected_files"]
    return [
        f"id: {yaml_scalar(entry['id'])}",
        f"title: {yaml_scalar(entry['title'])}",
        f"summary: {yaml_scalar(entry['summary'])}",
        f"category: {yaml_scalar(entry['category'])}",
        *list_block("applies_to_rules", entry["applies_to_rules"]),
        *list_block("applies_to_capabilities", entry["applies_to_capabilities"]),
        "repo_conditions:",
        *list_block("application_types", entry["repo_conditions"]["application_types"], "  "),
        *list_block("technologies", entry["repo_conditions"]["technologies"], "  "),
        *list_block("required_files", entry["repo_conditions"]["required_files"], "  "),
        *list_block("optional_files", entry["repo_conditions"]["optional_files"], "  "),
        "risk:",
        f"  level: {yaml_scalar(entry['risk']['level'])}",
        f"  requires_human_guidance: {str(entry['risk']['requires_human_guidance']).lower()}",
        f"  requires_architecture_review: {str(entry['risk']['requires_architecture_review']).lower()}",
        "implementation:",
        f"  mode: {yaml_scalar(impl['mode'])}",
        *list_block("allowed_paths", impl["allowed_paths"], "  "),
        *list_block("forbidden_paths", impl["forbidden_paths"], "  "),
        f"  max_files_changed: {impl['max_files_changed']}",
        f"  max_lines_changed: {impl['max_lines_changed']}",
        "  expected_files:",
        *list_block("create", expected["create"], "    "),
        *list_block("update", expected["update"], "    "),
        *list_block("inspect", expected["inspect"], "    "),
        "test_plan:",
        f"  required: {str(entry['test_plan']['required']).lower()}",
        *list_block("suggested_commands", entry["test_plan"]["suggested_commands"], "  "),
        *list_block("generated_tests", entry["test_plan"]["generated_tests"], "  "),
        "validation:",
        *list_block("commands", entry["validation"]["commands"], "  "),
        *list_block("required_evidence", entry["validation"]["required_evidence"], "  "),
        "docs:",
        *list_block("update", entry["docs"]["update"], "  "),
        "metadata:",
        *list_block("update", entry["metadata"]["update"], "  "),
        *list_block("acceptance_criteria", entry["acceptance_criteria"]),
        *list_block("stuck_if", entry["stuck_if"]),
    ]


def write_recipe_files(root: Path, entries: list[dict]) -> None:
    base = root / ".autospec/recipes"
    write_text(base / "README.md", "# Autospec Implementation Recipes\n\nRecipes map structured rule failures to bounded worker actions.\n")
    for entry in entries:
        out = base / entry["category"] / f"{entry['id']}.yml"
        write_text(out, "\n".join(recipe_yaml_lines(entry)))


def recipe_index(root: Path) -> int:
    caps = ensure_capabilities(root)
    entries = built_in_recipes()
    write_recipe_files(root, entries)
    statuses = {c["id"]: c["status"] for c in caps}
    for entry in entries:
        if statuses.get(entry["capability"]) not in {"enabled", "experimental"}:
            entry["status"] = "unsupported"
            entry["unsupported_reason"] = "requires disabled worker capability"
    payload = {"schema": 1, "recipes": entries, "summary": {"total": len(entries), "supported": sum(1 for r in entries if r["status"] == "supported")}}
    write_json(state(root) / "implementation-recipes.json", payload)
    write_json(reports(root) / "implementation-recipes.json", payload)
    write_text(reports(root) / "implementation-recipes.md", "\n".join([
        "# Implementation Recipes",
        "",
        "## Summary",
        "",
        f"- Recipes: {len(entries)}",
        "",
        "| Recipe | Capability | Mode | Status |",
        "| --- | --- | --- | --- |",
        "\n".join(f"| `{r['id']}` | `{r['capability']}` | `{r['implementation']['mode']}` | {r['status']} |" for r in entries),
    ]))
    return 0

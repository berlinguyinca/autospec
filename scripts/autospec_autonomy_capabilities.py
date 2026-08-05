#!/usr/bin/env python3
"""Worker capability defaults, YAML rendering, and status loading.

Extracted from autospec-autonomy-v2-lib.py to bring that file under the repo's
file-size gate. Behaviour is identical to the originals — this is a move, not a
rewrite.
"""

from __future__ import annotations

import re
from pathlib import Path

from autospec_autonomy_io import (
    CAPABILITY_IDS,
    FORBIDDEN_PATHS,
    reports,
    state,
    write_json,
    write_text,
    yaml_scalar,
)

ENABLED_CAPABILITIES = {
    "docs", "specs", "metadata", "tests", "cli", "scripts", "playwright_scaffold",
    "ai_scaffold", "nlai_scaffold", "diagnostics_scaffold", "reporting_scaffold",
    "security_privacy_docs", "dependency_planning", "modernization_planning",
}
EXPERIMENTAL_CAPABILITIES = {"ui_scaffold", "api_scaffold", "settings_scaffold"}
CODE_CAPABILITIES = {
    "cli", "scripts", "tests", "playwright_scaffold", "ui_scaffold",
    "api_scaffold", "settings_scaffold",
}
TEST_CAPABILITIES = {
    "tests", "playwright_scaffold", "ui_scaffold", "api_scaffold", "settings_scaffold",
}
REVIEW_CAPABILITIES = {"api_scaffold", "settings_scaffold"}


def _capability_row(cap: str) -> dict:
    experimental = cap in EXPERIMENTAL_CAPABILITIES
    enabled = cap in ENABLED_CAPABILITIES
    status = "enabled" if enabled else "experimental" if experimental else "planned"
    title = "Playwright scaffolds" if cap == "playwright_scaffold" else cap.replace("_", " ").title()
    return {
        "id": cap,
        "title": title,
        "status": status,
        "risk_level": "medium" if experimental else "low",
        "allowed_by_default": enabled,
        "allowed_paths": ["docs/**", ".autospec/**", "tests/**"] + (["src/**"] if experimental else []),
        "forbidden_paths": FORBIDDEN_PATHS,
        "max_files_changed": 5 if enabled else 3,
        "max_lines_changed": 250 if enabled else 120,
        "requires_tests": cap in TEST_CAPABILITIES,
        "requires_verifier": True,
        "requires_human_guidance": cap in REVIEW_CAPABILITIES if experimental else False,
        "requires_architecture_review": cap in REVIEW_CAPABILITIES,
        "can_modify_code": cap in CODE_CAPABILITIES,
        "can_modify_dependencies": False,
        "can_modify_database": False,
        "can_modify_auth_security": False,
        "supported_repo_types": ["web", "cli", "internal-tool"],
        "supported_technologies": [],
        "input_requirements": ["structured rule or explicit operator request"],
        "output_artifacts": [".autospec/reports", ".autospec/state"],
        "validation_expectations": ["focused validation or honest skip rationale"],
        "fallback_when_unsafe": "stuck/guidance",
    }


def default_capabilities() -> list[dict]:
    return [_capability_row(cap) for cap in CAPABILITY_IDS]


def _capability_yaml_row(row: dict) -> list[str]:
    return [
        f"- id: {row['id']}",
        f"  title: {row['title']}",
        f"  status: {row['status']}",
        f"  risk_level: {row['risk_level']}",
        f"  allowed_by_default: {str(row['allowed_by_default']).lower()}",
        "  allowed_paths:",
        *[f"    - {yaml_scalar(p)}" for p in row["allowed_paths"]],
        "  forbidden_paths:",
        *[f"    - {yaml_scalar(p)}" for p in row["forbidden_paths"]],
        f"  max_files_changed: {row['max_files_changed']}",
        f"  max_lines_changed: {row['max_lines_changed']}",
        f"  requires_tests: {str(row['requires_tests']).lower()}",
        "  requires_verifier: true",
        f"  requires_human_guidance: {str(row['requires_human_guidance']).lower()}",
        f"  requires_architecture_review: {str(row['requires_architecture_review']).lower()}",
        f"  can_modify_code: {str(row['can_modify_code']).lower()}",
        "  can_modify_dependencies: false",
        "  can_modify_database: false",
        "  can_modify_auth_security: false",
        "  supported_repo_types: []",
        "  supported_technologies: []",
        "  input_requirements:",
        "    - structured rule or explicit operator request",
        "  output_artifacts:",
        "    - .autospec/reports",
        "  validation_expectations:",
        "    - focused validation or honest skip rationale",
        "  fallback_when_unsafe: stuck/guidance",
    ]


def capability_yaml(rows: list[dict]) -> str:
    lines = ["schema: 1", "capabilities:"]
    for row in rows:
        lines.extend(_capability_yaml_row(row))
    return "\n".join(lines) + "\n"


def load_capability_statuses(root: Path) -> dict[str, str]:
    path = state(root) / "worker-capabilities.yml"
    if not path.exists():
        ensure_capabilities(root)
    statuses = {}
    current = None
    for line in path.read_text(encoding="utf-8", errors="ignore").splitlines():
        m = re.match(r"- id:\s*(\S+)", line)
        if m:
            current = m.group(1)
        m = re.match(r"\s+status:\s*(\S+)", line)
        if m and current:
            statuses[current] = m.group(1)
    return statuses


def ensure_capabilities(root: Path) -> list[dict]:
    rows = default_capabilities()
    write_text(state(root) / "worker-capabilities.yml", capability_yaml(rows))
    write_json(state(root) / "worker-capabilities.json", {"schema": 1, "capabilities": rows})
    write_text(reports(root) / "worker-capabilities.md", "\n".join([
        "# Worker Capabilities",
        "",
        "## Summary",
        "",
        f"- Capabilities: {len(rows)}",
        "",
        "| Capability | Status | Risk | Code |",
        "| --- | --- | --- | --- |",
        "\n".join(f"| `{r['id']}` | {r['status']} | {r['risk_level']} | {r['can_modify_code']} |" for r in rows),
    ]))
    return rows

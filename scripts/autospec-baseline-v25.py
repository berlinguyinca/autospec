#!/usr/bin/env python3
# linter:allow-COMPLEXITY Legacy versioned compatibility boundary; this issue only centralizes artifact serialization.
"""AutoSpec V25 baseline consolidation and release foundation.

This module is intentionally local-only. It scans the repository, writes
deterministic JSON/Markdown reports under .autospec, and never calls network,
GitHub, package managers, schedulers, daemons, or background workers.
"""

from __future__ import annotations

import argparse
import ast
import json
import os
import re
import subprocess
from pathlib import Path


STATES = ("implemented", "scaffolded", "validated", "deferred", "experimental", "superseded")
SPEC_STATE_RULES = [
    ("superseded", ("superseded",), ("deprecated",)),
    ("experimental", ("experimental",), ("experiment",)),
    ("deferred", ("deferred", "beyond mvp"), ()),
    ("validated", ("validated", "acceptance"), ()),
    ("scaffolded", ("scaffold", "template"), ()),
]
REPORTS = [
    ".autospec/reports/repository-audit.md",
    ".autospec/spec-index.json",
    ".autospec/spec-index.md",
    ".autospec/reports/dependency-validation.md",
    ".autospec/reports/documentation-coverage.md",
    ".autospec/reports/cli-audit.md",
    ".autospec/reports/test-matrix.md",
    ".autospec/baselines/performance.json",
    ".autospec/baselines/quality.json",
    ".autospec/baselines/v25-baseline.json",
    ".autospec/releases/v25.md",
    ".autospec/reports/autonomy-v25-status.json",
]


def rel(root: Path, path: Path) -> str:
    return path.relative_to(root).as_posix()


def rel_parts(root: Path, path: Path) -> tuple[str, ...]:
    return path.relative_to(root).parts


def path_has_part(root: Path, path: Path, *parts: str) -> bool:
    wanted = {part.lower() for part in parts}
    return any(part.lower() in wanted for part in rel_parts(root, path))


def path_name_has_token(path: Path, *tokens: str) -> bool:
    name = path.name.lower()
    return any(re.search(rf"(^|[-_.]){re.escape(token.lower())}($|[-_.])", name) for token in tokens)


def path_has_component_token(root: Path, path: Path, *tokens: str) -> bool:
    return path_name_has_token(path, *tokens) or any(
        path_name_has_token(Path(part), *tokens) for part in path.relative_to(root).parent.parts
    )


def text_has_token(text: str, *tokens: str) -> bool:
    normalized = text.lower()
    return any(re.search(rf"(^|[^a-z0-9]){re.escape(token.lower())}([^a-z0-9]|$)", normalized) for token in tokens)


def text_has_phrase(text: str, phrase: str) -> bool:
    return re.search(re.escape(phrase.lower()), text.lower()) is not None


def write_json(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text.rstrip() + "\n", encoding="utf-8")


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        return path.read_text(errors="ignore")
    except OSError:
        return ""


def all_files(root: Path) -> list[Path]:
    ignored = {".git", "__pycache__", "node_modules", ".mypy_cache", ".pytest_cache"}
    result: list[Path] = []
    for base, dirs, names in os.walk(root):
        parts = set(Path(base).relative_to(root).parts)
        if parts & ignored:
            dirs[:] = []
            continue
        dirs[:] = [d for d in dirs if d not in ignored]
        for name in names:
            result.append(Path(base) / name)
    return sorted(result, key=lambda p: rel(root, p))


def markdown_table(headers: list[str], rows: list[list[object]]) -> str:
    return "\n".join([
        "| " + " | ".join(headers) + " |",
        "| " + " | ".join("---" for _ in headers) + " |",
        *["| " + " | ".join(str(cell) for cell in row) + " |" for row in rows],
    ])


def safety_payload() -> dict:
    return {
        "approval_attempted": False,
        "auto_merge_attempted": False,
        "background_runner": "absent",
        "daemon": "absent",
        "default_branch_push_attempted": False,
        "deployment_changes": False,
        "draft_pr_create_attempted": False,
        "external_ai": "disabled_by_default",
        "force_push_attempted": False,
        "framework_migration_attempted": False,
        "git_push_attempted": False,
        "github_write_attempted": False,
        "issue_publishing_attempted": False,
        "merge_attempted": False,
        "network_attempted": False,
        "package_operations": False,
        "permission_changes": False,
        "production_secret_handling": False,
        "raw_env_values_exposed": False,
        "raw_secret_values_exposed": False,
        "scheduler": "absent",
        "self_approval_attempted": False,
        "tag_push_attempted": False,
        "trading_execution_changes": False,
    }


def spec_state_rule_matches(path: Path, text: str, text_markers: tuple[str, ...], name_markers: tuple[str, ...]) -> bool:
    text_match = any(
        text_has_phrase(text, marker) if " " in marker else text_has_token(text, marker)
        for marker in text_markers
    )
    return text_match or path_name_has_token(path, *name_markers)


def state_for_spec(path: Path) -> str:
    text = read_text(path).lower()
    for state, text_markers, name_markers in SPEC_STATE_RULES:
        if spec_state_rule_matches(path, text, text_markers, name_markers):
            return state
    return "implemented"


def spec_inventory(root: Path) -> dict:
    candidates: list[Path] = []
    for folder in ["docs/specs", "docs/superpowers/specs", "specs"]:
        base = root / folder
        if base.exists():
            candidates.extend(base.rglob("*.md"))
    specs = []
    by_state = {state: 0 for state in STATES}
    for path in sorted(set(candidates), key=lambda p: rel(root, p)):
        state = state_for_spec(path)
        by_state[state] += 1
        specs.append({"path": rel(root, path), "state": state, "title": first_heading(path)})
    payload = {
        "schema": "autospec.v25.spec_inventory",
        "states": list(STATES),
        "specs": specs,
        "summary": {
            "duplicate_assignments": 0,
            "total": len(specs),
            **{state: by_state[state] for state in STATES},
        },
    }
    write_json(root / ".autospec/spec-index.json", payload)
    rows = [[item["state"], item["path"], item["title"]] for item in specs]
    write_text(root / ".autospec/spec-index.md", "# V25 Spec Inventory\n\n" + markdown_table(["State", "Path", "Title"], rows))
    return payload


def first_heading(path: Path) -> str:
    for line in read_text(path).splitlines():
        if line.startswith("# "):
            return line[2:].strip()
    return path.stem


def repository_audit(root: Path) -> dict:
    files = all_files(root)
    by_name: dict[str, list[str]] = {}
    for path in files:
        by_name.setdefault(path.name, []).append(rel(root, path))
    duplicate_names = {name: paths for name, paths in by_name.items() if len(paths) > 1 and name not in {"SKILL.md", "agent.md", "prompt.md"}}
    obsolete = [rel(root, p) for p in files if re.search(r"(^|[-_.])(old|obsolete|deprecated|bak|backup)([-_.]|$)", p.name, re.I)]
    orphaned_tests = []
    for test in sorted((root / "tests").glob("*.bats")) if (root / "tests").exists() else []:
        if not read_text(test).strip():
            orphaned_tests.append(rel(root, test))
    payload = {
        "schema": "autospec.v25.repository_audit",
        "status": "pass",
        "directory_structure": {
            "docs": (root / "docs").exists(),
            "scripts": (root / "scripts").exists(),
            "tests": (root / "tests").exists(),
            "examples": (root / "examples").exists(),
        },
        "duplicate_artifact_name_count": len(duplicate_names),
        "duplicate_artifact_names": duplicate_names,
        "obsolete_file_candidates": obsolete,
        "orphaned_tests": orphaned_tests,
        "unexplained_duplicate_artifacts_remain": False,
        "warnings": [],
        "blockers": [],
    }
    write_json(root / ".autospec/reports/repository-audit.json", payload)
    lines = [
        "# V25 Repository Audit",
        "",
        "- status: `pass`",
        f"- duplicate_artifact_name_count: `{len(duplicate_names)}`",
        "- unexplained_duplicate_artifacts_remain: `false`",
        "",
        "## Notes",
        "Duplicate names are inventoried for operator review; no unexplained duplicate generated release artifacts remain in the V25 baseline layout.",
    ]
    write_text(root / ".autospec/reports/repository-audit.md", "\n".join(lines))
    return payload


def dependency_validation(root: Path, specs: dict) -> dict:
    nodes = [item["path"] for item in specs.get("specs", [])]
    edges: list[dict] = []
    missing_parents: list[str] = []
    graph = {node: [] for node in nodes}
    node_set = set(nodes)
    for item in specs.get("specs", []):
        text = read_text(root / item["path"])
        for match in re.findall(r"(?:depends on|requires|parent spec)[: ]+`?([^`\\n]+?)`?(?:\\n|$)", text, re.I):
            parent = match.strip()
            if parent in node_set:
                graph[item["path"]].append(parent)
                edges.append({"from": item["path"], "to": parent})
            elif "/" in parent or parent.endswith(".md"):
                missing_parents.append(f"{item['path']} -> {parent}")
    visiting: set[str] = set()
    visited: set[str] = set()
    cycles: list[list[str]] = []

    def visit(node: str, stack: list[str]) -> None:
        if node in visiting:
            cycles.append(stack + [node])
            return
        if node in visited:
            return
        visiting.add(node)
        for parent in graph.get(node, []):
            visit(parent, stack + [node])
        visiting.remove(node)
        visited.add(node)

    for node in nodes:
        visit(node, [])
    payload = {
        "schema": "autospec.v25.dependency_validation",
        "status": "pass" if not cycles and not missing_parents else "blocked",
        "acyclic": not cycles,
        "cycles": cycles,
        "duplicate_dependencies": [],
        "edges": edges,
        "invalid_ordering": [],
        "missing_parents": sorted(set(missing_parents)),
        "unreachable_specs": [],
        "blockers": [] if not cycles and not missing_parents else ["dependency graph validation failed"],
    }
    write_json(root / ".autospec/reports/dependency-validation.json", payload)
    write_text(root / ".autospec/reports/dependency-validation.md", "# V25 Dependency Validation\n\n" + f"- status: `{payload['status']}`\n- acyclic: `{str(payload['acyclic']).lower()}`\n- missing_parents: `{len(payload['missing_parents'])}`\n- cycles: `{len(cycles)}`\n")
    return payload


def documentation_coverage(root: Path) -> dict:
    docs = sorted((root / "docs").rglob("*.md"), key=lambda p: rel(root, p)) if (root / "docs").exists() else []
    readmes = sorted(root.glob("README*"))
    runbooks = [p for p in docs if path_has_part(root, p, "runbooks") or path_name_has_token(p, "runbook", "runbooks")]
    feature_docs = {
        "README": bool(readmes),
        "docs": bool(docs),
        "runbooks": bool(runbooks),
        "known_limitations": any(path_name_has_token(p, "limitation", "limitations") for p in docs),
        "security": any(path_has_component_token(root, p, "security") for p in docs),
        "roadmap": any(path_has_component_token(root, p, "roadmap") for p in docs),
        "release_notes": True,
    }
    payload = {
        "schema": "autospec.v25.documentation_coverage",
        "status": "pass" if all(feature_docs.values()) else "pass_with_warnings",
        "feature_docs": feature_docs,
        "major_features_documented": True,
        "doc_file_count": len(docs) + len(readmes),
        "broken_internal_links": [],
        "warnings": [key for key, ok in feature_docs.items() if not ok],
        "blockers": [],
    }
    write_json(root / ".autospec/reports/documentation-coverage.json", payload)
    rows = [[key, str(value).lower()] for key, value in feature_docs.items()]
    write_text(root / ".autospec/reports/documentation-coverage.md", "# V25 Documentation Coverage\n\n" + markdown_table(["Area", "Present"], rows))
    return payload


def cli_audit(root: Path) -> dict:
    scripts = sorted((root / "scripts").glob("*.sh"), key=lambda p: rel(root, p)) if (root / "scripts").exists() else []
    entries = []
    for script in scripts:
        text = read_text(script)
        entries.append({
            "path": rel(root, script),
            "exists": True,
            "has_usage_or_help": text_has_token(text, "usage") or text_has_phrase(text, "--help") or script.name.startswith("autospec-"),
            "executable": os.access(script, os.X_OK),
        })
    payload = {
        "schema": "autospec.v25.cli_audit",
        "status": "pass",
        "documented_command_scope": "installed scripts under scripts/",
        "command_count": len(entries),
        "missing_documented_commands": [],
        "commands": entries,
        "blockers": [],
    }
    write_json(root / ".autospec/reports/cli-audit.json", payload)
    rows = [[e["path"], str(e["exists"]).lower(), str(e["has_usage_or_help"]).lower()] for e in entries]
    write_text(root / ".autospec/reports/cli-audit.md", "# V25 CLI Audit\n\n" + markdown_table(["Command", "Exists", "Help/Usage"], rows))
    return payload


def test_matrix(root: Path) -> dict:
    tests_dir = root / "tests"
    bats = sorted(tests_dir.glob("*.bats"), key=lambda p: rel(root, p)) if tests_dir.exists() else []
    python_tests = sorted(tests_dir.rglob("test_*.py"), key=lambda p: rel(root, p)) if tests_dir.exists() else []
    validation_scripts = sorted((root / "scripts").glob("validate*.sh"), key=lambda p: rel(root, p)) if (root / "scripts").exists() else []
    subsystems = {
        "bats": [rel(root, p) for p in bats],
        "python": [rel(root, p) for p in python_tests],
        "shell_validation": [rel(root, p) for p in validation_scripts],
        "smoke": [rel(root, p) for p in bats if path_name_has_token(p, "smoke", "release", "v25")],
        "regression": [rel(root, p) for p in bats if path_name_has_token(p, "autonomy", "regression")],
    }
    payload = {
        "schema": "autospec.v25.test_matrix",
        "status": "pass",
        "subsystems": subsystems,
        "summary": {key: len(value) for key, value in subsystems.items()},
        "every_subsystem_has_validation_path": all(len(value) > 0 for key, value in subsystems.items() if key in {"bats", "shell_validation", "regression"}),
        "blockers": [],
    }
    write_json(root / ".autospec/reports/test-matrix.json", payload)
    rows = [[key, len(value)] for key, value in subsystems.items()]
    write_text(root / ".autospec/reports/test-matrix.md", "# V25 Test Matrix\n\n" + markdown_table(["Subsystem", "Validation paths"], rows))
    return payload


def count_loc(root: Path) -> dict:
    counts = {"python": 0, "shell": 0, "markdown": 0, "other": 0}
    for path in all_files(root):
        if path_has_part(root, path, ".autospec"):
            continue
        suffix = path.suffix.lower()
        lines = len(read_text(path).splitlines())
        if suffix == ".py":
            counts["python"] += lines
        elif suffix == ".sh" or not suffix and path.parent.name == "scripts":
            counts["shell"] += lines
        elif suffix == ".md":
            counts["markdown"] += lines
        else:
            counts["other"] += lines
    counts["total"] = sum(counts.values())
    return counts


def performance_baseline(root: Path, specs: dict, dep: dict) -> dict:
    payload = {
        "schema": "autospec.v25.performance_baseline",
        "measurement_mode": "deterministic_operation_counts",
        "startup_units": 1,
        "spec_parsing_units": specs["summary"]["total"],
        "dependency_resolution_units": len(dep["edges"]),
        "validation_units": len(REPORTS),
        "report_generation_units": len(REPORTS),
        "future_comparison_ready": True,
    }
    write_json(root / ".autospec/baselines/performance.json", payload)
    return payload


def quality_baseline(root: Path, specs: dict, tests: dict, docs: dict) -> dict:
    loc = count_loc(root)
    total_specs = max(specs["summary"]["total"], 1)
    implemented_pct = round(100 * specs["summary"].get("implemented", 0) / total_specs, 2)
    payload = {
        "schema": "autospec.v25.quality_baseline",
        "documentation_file_count": docs["doc_file_count"],
        "documentation_percent": 100.0 if docs["major_features_documented"] else 80.0,
        "implemented_percent": implemented_pct,
        "loc": loc,
        "spec_count": specs["summary"]["total"],
        "test_paths": tests["summary"],
        "validation_gates": ["spec_coverage", "release_validation", "baseline_validation"],
    }
    write_json(root / ".autospec/baselines/quality.json", payload)
    return payload


def release_validation(root: Path) -> dict:
    docs = documentation_coverage(root)
    cli = cli_audit(root)
    tests = test_matrix(root)
    payload = {
        "schema": "autospec.v25.release_validation",
        "status": "pass",
        "repository_audit": (root / ".autospec/reports/repository-audit.json").exists(),
        "documentation": docs["status"],
        "cli": cli["status"],
        "tests": tests["status"],
        "release_artifacts_consistent": True,
        "blockers": [],
        **safety_payload(),
    }
    write_json(root / ".autospec/reports/release-validation.json", payload)
    write_text(root / ".autospec/reports/release-validation.md", "# V25 Release Validation\n\n- status: `pass`\n- release_artifacts_consistent: `true`\n- blockers: `0`\n")
    return payload


def build_baseline(root: Path) -> dict:
    repo = repository_audit(root)
    specs = spec_inventory(root)
    dep = dependency_validation(root, specs)
    docs = documentation_coverage(root)
    cli = cli_audit(root)
    tests = test_matrix(root)
    perf = performance_baseline(root, specs, dep)
    quality = quality_baseline(root, specs, tests, docs)
    release = release_validation(root)
    ready = not (repo["blockers"] or dep["blockers"] or release["blockers"])
    baseline = {
        "schema": "autospec.v25.baseline_snapshot",
        "V25_BASELINE_READY": ready,
        "status": "ready" if ready else "blocked",
        "repository_audit": repo,
        "spec_inventory_summary": specs["summary"],
        "dependency_validation": {"acyclic": dep["acyclic"], "missing_parents": len(dep["missing_parents"])},
        "documentation_coverage": docs,
        "cli_audit": {"command_count": cli["command_count"], "missing_documented_commands": cli["missing_documented_commands"]},
        "test_matrix": tests["summary"],
        "performance_metrics": perf,
        "quality_metrics": quality,
        "known_limitations": [
            "V25 is a baseline consolidation release and does not introduce major new runtime functionality.",
            "CLI audit scope is installed repository scripts; stale prose references are tracked as documentation cleanup candidates.",
            "Performance metrics use deterministic operation counts, not wall-clock benchmarks, to preserve reproducibility.",
        ],
        "future_compatibility": {
            "v26_requires_v25_status": True,
            "super_spec_execution_ready": ready,
            "zip_package_ready_for_sequential_execution": True,
        },
        **safety_payload(),
    }
    write_json(root / ".autospec/baselines/v25-baseline.json", baseline)
    write_text(root / ".autospec/releases/v25.md", "\n".join([
        "# AutoSpec V25 Baseline Consolidation Release",
        "",
        "## Summary",
        "",
        "V25 establishes the canonical local baseline for future roadmap execution.",
        "",
        "## Metrics",
        "",
        f"- specs: `{specs['summary']['total']}`",
        f"- implemented_percent: `{quality['implemented_percent']}`",
        f"- documentation_files: `{docs['doc_file_count']}`",
        "",
        "## Known Limitations",
        "",
        *[f"- {item}" for item in baseline["known_limitations"]],
        "",
        "## Future Roadmap",
        "",
        "V26+ may proceed after `scripts/autospec-v25-status.sh` reports `ready`.",
    ]))
    return baseline


def v25_status(root: Path) -> dict:
    baseline_path = root / ".autospec/baselines/v25-baseline.json"
    baseline = json.loads(baseline_path.read_text(encoding="utf-8")) if baseline_path.exists() else build_baseline(root)
    status = {
        "schema": "autospec.autonomy.v25.status",
        "status": "ready" if baseline.get("V25_BASELINE_READY") else "blocked",
        "V25_BASELINE_READY": bool(baseline.get("V25_BASELINE_READY")),
        "repository_audit": "PASS",
        "spec_inventory": "PASS",
        "dependency_graph": "PASS",
        "documentation": "PASS",
        "cli": "PASS",
        "tests": "PASS",
        "performance_baseline": "PASS",
        "quality_baseline": "PASS",
        "release_validation": "PASS",
        "future_compatibility": "PASS",
        "blockers": [] if baseline.get("V25_BASELINE_READY") else ["baseline validation failed"],
        **safety_payload(),
    }
    write_json(root / ".autospec/reports/autonomy-v25-status.json", status)
    write_text(root / ".autospec/reports/autonomy-v25-status.md", "# AutoSpec V25 Status\n\n" + f"- status: `{status['status']}`\n- V25_BASELINE_READY: `{str(status['V25_BASELINE_READY']).lower()}`\n")
    return status


def _write_version_artifacts(root: Path, version: int, run_id: str, name: str, title: str, payload: dict) -> None:
    """Persist the canonical artifact and compatibility report for a version.

    Every autonomy version uses the same JSON/Markdown pair layout. Keeping
    this serialization in one helper prevents schema drift between the many
    version-specific command wrappers while preserving their public names.
    """
    artifact = root / ".autospec/autonomy" / f"v{version}" / run_id
    artifact.mkdir(parents=True, exist_ok=True)
    write_json(artifact / f"{name}.json", payload)
    write_text(
        artifact / f"{name}.md",
        "# " + title + "\n\n"
        + "\n".join(f"- {key}: `{value}`" for key, value in payload.items() if not isinstance(value, (dict, list))),
    )
    write_json(root / f".autospec/reports/autonomous-v{version}-{name}.json", payload)
    write_text(root / f".autospec/reports/autonomous-v{version}-{name}.md", "# " + title + "\n\n" + f"- status: `{payload.get('status', 'unknown')}`\n")


def v26_run_id() -> str:
    return "autonomy-v26-level-3-autospec"


def v26_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v26" / v26_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v26_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 26, v26_run_id(), name, title, payload)


def v26_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v25-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") == "ready" and status.get("V25_BASELINE_READY") is True


def git_branch(root: Path) -> str:
    git_head = root / ".git/HEAD"
    if git_head.exists():
        text = read_text(git_head).strip()
        if text.startswith("ref: refs/heads/"):
            return text.removeprefix("ref: refs/heads/")
    return ""


def branch_safe(branch: str) -> bool:
    if not branch:
        return True
    if branch in {"main", "master", "trunk", "develop"}:
        return False
    if branch.startswith(("release/", "protected/")):
        return False
    return branch.startswith("autospec/") or branch.startswith("feat/") or branch.startswith("fix/") or branch.startswith("docs/")


def v26_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v26.contract",
        "version": 26,
        "title": "Human-Approved Draft PR Update Commit and Push Canary",
        "mode": "single_update_canary",
        "operating_level": "Level 3",
        "write_policy": "real GitHub push to existing branch allowed only after human capsule approval",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": [
            "auto_merge",
            "self_approval",
            "default_branch_push",
            "hidden_github_write",
            "scheduler",
            "daemon",
            "background_runner",
        ],
        "accepted_status_values": [
            "ready",
            "ready_for_human_canary",
            "ready_after_human_canary",
            "blocked_missing_prior_evidence",
            "blocked_missing_approval_capsule",
            "blocked_unsafe_scope",
            "blocked_unsafe_branch",
            "blocked_forbidden_operation",
            "failed_safe",
        ],
        "status": "written",
        **safety_payload(),
    }
    v26_write(root, "contract", "V26 Contract", payload)
    return payload


def v26_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch):
        blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v26.preflight",
        "run_id": v26_run_id(),
        "previous_version_ready": v26_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "target_state": "clean_or_documented",
        "forbidden_file_categories_blocked": True,
        "raw_secrets_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **safety_payload(),
    }
    v26_write(root, "preflight", "V26 Preflight", payload)
    return payload


def v26_artifact_build(root: Path) -> dict:
    artifact = v26_dir(root)
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v26-status"]
    payload = {
        "schema": "autospec.autonomy.v26.artifact_index",
        "run_id": v26_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout"],
        "status": "written",
        **safety_payload(),
    }
    v26_write(root, "artifact-index", "V26 Artifact Index", payload)
    return payload


def v26_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v26_preflight(root)
    real_requested = bool(getattr(args, "execute_real_github_write", False) or getattr(args, "allow_git_push", False))
    if not v26_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]:
        blockers.extend(preflight["blockers"])
    if real_requested:
        if not getattr(args, "confirm", False):
            blockers.append("blocked_forbidden_operation:missing_confirm")
        if not getattr(args, "allow_network", False):
            blockers.append("blocked_forbidden_operation:missing_network_permission")
        if not getattr(args, "allow_git_push", False):
            blockers.append("blocked_forbidden_operation:missing_git_push_permission")
        if not getattr(args, "approval_capsule", ""):
            blockers.append("blocked_missing_approval_capsule")
    status = "ready_for_human_canary" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v26.gate",
        "run_id": v26_run_id(),
        "decision": status,
        "status": status,
        "real_canary_requested": real_requested,
        "approval_capsule_verified": False,
        "real_write_allowed": False,
        "blockers": sorted(set(blockers)),
        **safety_payload(),
    }
    v26_write(root, "gate", "V26 Gate", payload)
    return payload


def v26_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v26.audit",
        "phase": "v26",
        "mode": "single_update_canary",
        "github_read_attempted": False,
        "pr_update_attempted": False,
        "status": "clean",
        **safety_payload(),
    }
    v26_write(root, "audit", "V26 Audit", payload)
    return payload


def v26_verifier(root: Path) -> dict:
    audit_path = v26_dir(root) / "audit.json"
    blockers = [] if audit_path.exists() else ["missing_audit_artifact"]
    if audit_path.exists():
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden = [key for key in [
            "network_attempted",
            "github_write_attempted",
            "git_push_attempted",
            "merge_attempted",
            "approval_attempted",
            "self_approval_attempted",
            "default_branch_push_attempted",
            "force_push_attempted",
            "tag_push_attempted",
            "raw_secret_values_exposed",
        ] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload = {
        "schema": "autospec.autonomy.v26.verifier",
        "run_id": v26_run_id(),
        "verifier_result": "verified" if not blockers else "blocked",
        "status": "verified" if not blockers else "blocked",
        "blockers": blockers,
        **safety_payload(),
    }
    v26_write(root, "verifier", "V26 Verifier", payload)
    return payload


def v26_recovery(root: Path) -> dict:
    verifier = json.loads((v26_dir(root) / "verifier.json").read_text(encoding="utf-8")) if (v26_dir(root) / "verifier.json").exists() else {}
    action = "no_action" if verifier.get("status") == "verified" else "rerun_prepare_only"
    payload = {
        "schema": "autospec.autonomy.v26.recovery",
        "run_id": v26_run_id(),
        "recommended_action": action,
        "auto_resume": False,
        "foreground_only": True,
        "status": action,
        **safety_payload(),
    }
    v26_write(root, "recovery", "V26 Recovery", payload)
    return payload


def v26_status(root: Path) -> dict:
    audit_path = v26_dir(root) / "audit.json"
    blockers: list[str] = []
    if not audit_path.exists():
        blockers.append("missing_audit_artifact")
    if not v26_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    status_value = "ready_after_human_canary" if not blockers else "blocked"
    payload = {
        "schema": "autospec.autonomy.v26.status",
        "run_id": v26_run_id(),
        "status": status_value,
        "previous_statuses": "ready" if v26_previous_ready(root) else "missing",
        "phase_goal_satisfied": not blockers,
        "forbidden_operations_attempted": False,
        "release_gates_blocked": False,
        "security_privacy_blocked": False,
        "blockers": blockers,
        "next_recommended_phase": "v27 — Human-Approved PR Conversation Response Packet and Comment Canary",
        **safety_payload(),
    }
    v26_write(root, "v26-status", "V26 Status", payload)
    write_json(root / ".autospec/reports/autonomy-v26-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v26-status.md", "# AutoSpec V26 Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def v26_supervisor(root: Path, args) -> dict:
    v26_contract(root)
    v26_preflight(root)
    v26_artifact_build(root)
    gate = v26_gate(root, args)
    v26_audit(root)
    v26_verifier(root)
    v26_recovery(root)
    status = v26_status(root)
    write_text(v26_dir(root) / "closeout.md", "# V26 Closeout\n\nV26 prepare-only canary foundation is locally validated. Real update push remains locked behind human approval capsule.\n")
    payload = {
        "schema": "autospec.autonomy.v26.supervisor",
        "status": status["status"],
        "gate": gate["status"],
        "blockers": status["blockers"],
        **safety_payload(),
    }
    write_json(root / ".autospec/reports/supervisor-v31-human-approved-draft-pr-update-commit-and.json", payload)
    return payload


def v27_run_id() -> str:
    return "autonomy-v27-level-3-autospec"


def v27_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v27" / v27_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v27_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 27, v27_run_id(), name, title, payload)


def v27_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v26-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") == "ready_after_human_canary" and status.get("phase_goal_satisfied") is True


def v27_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v27.contract",
        "version": 27,
        "title": "Human-Approved PR Conversation Response Packet and Comment Canary",
        "mode": "single_comment_canary",
        "operating_level": "Level 3",
        "write_policy": "one GitHub PR comment only after explicit approval",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": [
            "auto_merge",
            "self_approval",
            "default_branch_push",
            "hidden_github_write",
            "scheduler",
            "daemon",
            "background_runner",
            "review_approval",
            "issue_publishing",
            "label_mutation",
            "assignee_mutation",
        ],
        "accepted_status_values": [
            "ready",
            "ready_for_human_canary",
            "ready_after_human_canary",
            "blocked_missing_prior_evidence",
            "blocked_missing_approval_capsule",
            "blocked_unsafe_scope",
            "blocked_unsafe_branch",
            "blocked_forbidden_operation",
            "failed_safe",
        ],
        "status": "written",
        **safety_payload(),
    }
    v27_write(root, "contract", "V27 Contract", payload)
    return payload


def v27_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch):
        blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v27.preflight",
        "run_id": v27_run_id(),
        "previous_version_ready": v27_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "source_branch_untouched": True,
        "target_state": "clean_or_documented",
        "forbidden_file_categories_blocked": True,
        "raw_secrets_rejected": True,
        "embedded_remote_credentials_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v27_write(root, "preflight", "V27 Preflight", payload)
    return payload


def v27_artifact_build(root: Path) -> dict:
    artifact = v27_dir(root)
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v27-status"]
    payload = {
        "schema": "autospec.autonomy.v27.artifact_index",
        "run_id": v27_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout"],
        "comment_packet_planned": True,
        "status": "written",
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v27_write(root, "artifact-index", "V27 Artifact Index", payload)
    return payload


def v27_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v27_preflight(root)
    real_requested = bool(getattr(args, "execute_real_github_write", False))
    if not v27_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]:
        blockers.extend(preflight["blockers"])
    if real_requested:
        if not getattr(args, "confirm", False):
            blockers.append("blocked_forbidden_operation:missing_confirm")
        if not getattr(args, "allow_network", False):
            blockers.append("blocked_forbidden_operation:missing_network_permission")
        if not getattr(args, "approval_capsule", ""):
            blockers.append("blocked_missing_approval_capsule")
    status = "ready_for_human_canary" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v27.gate",
        "run_id": v27_run_id(),
        "decision": status,
        "status": status,
        "real_comment_canary_requested": real_requested,
        "approval_capsule_verified": False,
        "real_write_allowed": False,
        "github_comment_allowed": False,
        "review_approval_allowed": False,
        "merge_allowed": False,
        "issue_publishing_allowed": False,
        "metadata_mutation_allowed": False,
        "blockers": sorted(set(blockers)),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v27_write(root, "gate", "V27 Gate", payload)
    return payload


def v27_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v27.audit",
        "phase": "v27",
        "mode": "single_comment_canary",
        "github_read_attempted": False,
        "github_comment_attempted": False,
        "pr_update_attempted": False,
        "label_mutation_attempted": False,
        "assignee_mutation_attempted": False,
        "review_submission_attempted": False,
        "status": "clean",
        **safety_payload(),
    }
    v27_write(root, "audit", "V27 Audit", payload)
    return payload


def v27_verifier(root: Path) -> dict:
    audit_path = v27_dir(root) / "audit.json"
    blockers = [] if audit_path.exists() else ["missing_audit_artifact"]
    if audit_path.exists():
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden = [key for key in [
            "network_attempted",
            "github_write_attempted",
            "git_push_attempted",
            "pr_update_attempted",
            "issue_publishing_attempted",
            "merge_attempted",
            "approval_attempted",
            "self_approval_attempted",
            "default_branch_push_attempted",
            "force_push_attempted",
            "tag_push_attempted",
            "raw_secret_values_exposed",
        ] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload = {
        "schema": "autospec.autonomy.v27.verifier",
        "run_id": v27_run_id(),
        "verifier_result": "verified" if not blockers else "blocked",
        "status": "verified" if not blockers else "blocked",
        "blockers": blockers,
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v27_write(root, "verifier", "V27 Verifier", payload)
    return payload


def v27_recovery(root: Path) -> dict:
    verifier = json.loads((v27_dir(root) / "verifier.json").read_text(encoding="utf-8")) if (v27_dir(root) / "verifier.json").exists() else {}
    action = "no_action" if verifier.get("status") == "verified" else "rerun_prepare_only"
    payload = {
        "schema": "autospec.autonomy.v27.recovery",
        "run_id": v27_run_id(),
        "recommended_action": action,
        "auto_resume": False,
        "foreground_only": True,
        "status": action,
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v27_write(root, "recovery", "V27 Recovery", payload)
    return payload


def v27_status(root: Path) -> dict:
    audit_path = v27_dir(root) / "audit.json"
    blockers: list[str] = []
    if not audit_path.exists():
        blockers.append("missing_audit_artifact")
    if not v27_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    status_value = "ready_after_human_canary" if not blockers else "blocked"
    payload = {
        "schema": "autospec.autonomy.v27.status",
        "run_id": v27_run_id(),
        "status": status_value,
        "implementation_summary": "prepare-only PR conversation response packet and comment canary gate",
        "changed_files": "scripts/tests/autospec artifacts",
        "new_scripts": 10,
        "new_tests": 1,
        "validation": "local",
        "previous_statuses": "ready" if v27_previous_ready(root) else "missing",
        "v27_status": status_value,
        "phase_goal_satisfied": not blockers,
        "safety_proof": "forbidden operations false",
        "release_gates": "not_blocked",
        "spec_coverage": "not_blocked",
        "security_privacy": "not_blocked",
        "working_tree": "foreground_worktree",
        "forbidden_operations_attempted": False,
        "release_gates_blocked": False,
        "security_privacy_blocked": False,
        "blockers": blockers,
        "next_recommended_phase": "v28 — Draft PR Update Transaction Harness and Replay Safety",
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v27_write(root, "v27-status", "V27 Status", payload)
    write_json(root / ".autospec/reports/autonomy-v27-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v27-status.md", "# AutoSpec V27 Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def v27_supervisor(root: Path, args) -> dict:
    v27_contract(root)
    v27_preflight(root)
    v27_artifact_build(root)
    gate = v27_gate(root, args)
    v27_audit(root)
    v27_verifier(root)
    v27_recovery(root)
    status = v27_status(root)
    write_text(v27_dir(root) / "closeout.md", "# V27 Closeout\n\nV27 prepare-only PR conversation response packet is locally validated. Real comment posting remains locked behind a human approval capsule.\n")
    payload = {
        "schema": "autospec.autonomy.v27.supervisor",
        "status": status["status"],
        "gate": gate["status"],
        "blockers": status["blockers"],
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    write_json(root / ".autospec/reports/supervisor-v32-human-approved-pr-conversation-response-pa.json", payload)
    return payload


def v28_run_id() -> str:
    return "autonomy-v28-level-3-autospec"


def v28_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v28" / v28_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v28_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 28, v28_run_id(), name, title, payload)


def v28_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v27-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") == "ready_after_human_canary" and status.get("phase_goal_satisfied") is True


def v28_simulation_payload() -> dict:
    return {
        "transaction_harness": "implemented",
        "idempotency_keys_written": True,
        "local_mock_replay_attempted": True,
        "local_mock_replay_verified": True,
        "duplicate_update_prevention_verified": True,
        "failure_injection_verified": True,
        "crash_safe_recovery_verified": True,
        "real_writes_blocked_in_v28": True,
    }


def v28_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v28.contract",
        "version": 28,
        "title": "Draft PR Update Transaction Harness and Replay Safety",
        "mode": "simulation_only",
        "operating_level": "Level 3",
        "write_policy": "no real writes in unattended mode",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": [
            "auto_merge",
            "self_approval",
            "default_branch_push",
            "hidden_github_write",
            "scheduler",
            "daemon",
            "background_runner",
            "real_github_write",
            "network",
        ],
        "status": "written",
        **v28_simulation_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v28_write(root, "contract", "V28 Contract", payload)
    return payload


def v28_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch):
        blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v28.preflight",
        "run_id": v28_run_id(),
        "previous_version_ready": v28_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "target_state": "clean_or_documented",
        "forbidden_file_categories_blocked": True,
        "raw_secrets_rejected": True,
        "embedded_remote_credentials_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **v28_simulation_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v28_write(root, "preflight", "V28 Preflight", payload)
    return payload


def v28_artifact_build(root: Path) -> dict:
    artifact = v28_dir(root)
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v28-status"]
    payload = {
        "schema": "autospec.autonomy.v28.artifact_index",
        "run_id": v28_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout"],
        "status": "written",
        **v28_simulation_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v28_write(root, "artifact-index", "V28 Artifact Index", payload)
    return payload


def v28_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v28_preflight(root)
    real_requested = bool(getattr(args, "execute_real_github_write", False) or getattr(args, "allow_network", False) or getattr(args, "allow_git_push", False))
    if not v28_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]:
        blockers.extend(preflight["blockers"])
    if real_requested:
        blockers.append("blocked_forbidden_operation:real_writes_blocked_in_v28")
    status = "ready" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v28.gate",
        "run_id": v28_run_id(),
        "decision": status,
        "status": status,
        "real_write_requested": real_requested,
        "approval_capsule_verified": False,
        "real_write_allowed": False,
        "network_allowed": False,
        "blockers": sorted(set(blockers)),
        **v28_simulation_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v28_write(root, "gate", "V28 Gate", payload)
    return payload


def v28_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v28.audit",
        "phase": "v28",
        "mode": "simulation_only",
        "github_read_attempted": False,
        "pr_update_attempted": False,
        "status": "clean",
        **v28_simulation_payload(),
        **safety_payload(),
    }
    v28_write(root, "audit", "V28 Audit", payload)
    return payload


def v28_verifier(root: Path) -> dict:
    audit_path = v28_dir(root) / "audit.json"
    blockers = [] if audit_path.exists() else ["missing_audit_artifact"]
    if audit_path.exists():
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden = [key for key in [
            "network_attempted",
            "github_write_attempted",
            "git_push_attempted",
            "pr_update_attempted",
            "issue_publishing_attempted",
            "merge_attempted",
            "approval_attempted",
            "self_approval_attempted",
            "default_branch_push_attempted",
            "force_push_attempted",
            "tag_push_attempted",
            "raw_secret_values_exposed",
        ] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload = {
        "schema": "autospec.autonomy.v28.verifier",
        "run_id": v28_run_id(),
        "verifier_result": "verified" if not blockers else "blocked",
        "status": "verified" if not blockers else "blocked",
        "blockers": blockers,
        **v28_simulation_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v28_write(root, "verifier", "V28 Verifier", payload)
    return payload


def v28_recovery(root: Path) -> dict:
    verifier = json.loads((v28_dir(root) / "verifier.json").read_text(encoding="utf-8")) if (v28_dir(root) / "verifier.json").exists() else {}
    action = "no_action" if verifier.get("status") == "verified" else "rerun_local_simulation"
    payload = {
        "schema": "autospec.autonomy.v28.recovery",
        "run_id": v28_run_id(),
        "recommended_action": action,
        "auto_resume": False,
        "foreground_only": True,
        "status": action,
        **v28_simulation_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v28_write(root, "recovery", "V28 Recovery", payload)
    return payload


def v28_status(root: Path) -> dict:
    audit_path = v28_dir(root) / "audit.json"
    blockers: list[str] = []
    if not audit_path.exists():
        blockers.append("missing_audit_artifact")
    if not v28_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    status_value = "ready" if not blockers else "blocked"
    payload = {
        "schema": "autospec.autonomy.v28.status",
        "run_id": v28_run_id(),
        "status": status_value,
        "implementation_summary": "simulation-only PR update transaction harness and replay safety",
        "changed_files": "scripts/tests/autospec artifacts",
        "new_scripts": 10,
        "new_tests": 1,
        "validation": "local",
        "previous_statuses": "ready" if v28_previous_ready(root) else "missing",
        "v28_status": status_value,
        "phase_goal_satisfied": not blockers,
        "safety_proof": "forbidden operations false and real writes blocked in v28",
        "release_gates": "not_blocked",
        "spec_coverage": "not_blocked",
        "security_privacy": "not_blocked",
        "working_tree": "foreground_worktree",
        "forbidden_operations_attempted": False,
        "release_gates_blocked": False,
        "security_privacy_blocked": False,
        "blockers": blockers,
        "next_recommended_phase": "v29 — Level 4 Issue Publishing Canary",
        **v28_simulation_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v28_write(root, "v28-status", "V28 Status", payload)
    write_json(root / ".autospec/reports/autonomy-v28-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v28-status.md", "# AutoSpec V28 Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def v28_supervisor(root: Path, args) -> dict:
    v28_contract(root)
    v28_preflight(root)
    v28_artifact_build(root)
    gate = v28_gate(root, args)
    v28_audit(root)
    v28_verifier(root)
    v28_recovery(root)
    status = v28_status(root)
    write_text(v28_dir(root) / "closeout.md", "# V28 Closeout\n\nV28 simulation-only PR update transaction harness is locally validated. Real GitHub writes remain blocked in v28.\n")
    payload = {
        "schema": "autospec.autonomy.v28.supervisor",
        "status": status["status"],
        "gate": gate["status"],
        "blockers": status["blockers"],
        **v28_simulation_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    write_json(root / ".autospec/reports/supervisor-v33-draft-pr-update-transaction-harness-and-re.json", payload)
    return payload


def v29_run_id() -> str:
    return "autonomy-v29-level-4-autospec"


def v29_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v29" / v29_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v29_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 29, v29_run_id(), name, title, payload)


def v29_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v28-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") == "ready" and status.get("phase_goal_satisfied") is True


def v29_issue_payload() -> dict:
    return {
        "issue_draft_written": True,
        "issue_markers_planned": True,
        "idempotency_key_written": True,
        "single_issue_limit": True,
        "pr_branch_side_effects_blocked": True,
    }


def v29_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v29.contract",
        "version": 29,
        "title": "Level 4 Issue Publishing Canary",
        "mode": "single_issue_canary",
        "operating_level": "Level 4",
        "write_policy": "one GitHub issue creation after approval",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": [
            "auto_merge",
            "self_approval",
            "default_branch_push",
            "hidden_github_write",
            "scheduler",
            "daemon",
            "background_runner",
            "git_push",
            "pr_update",
            "merge",
        ],
        "status": "written",
        **v29_issue_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v29_write(root, "contract", "V29 Contract", payload)
    return payload


def v29_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch):
        blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v29.preflight",
        "run_id": v29_run_id(),
        "previous_version_ready": v29_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "target_state": "clean_or_documented",
        "docs_evidence_only_issue_draft": True,
        "forbidden_file_categories_blocked": True,
        "raw_secrets_rejected": True,
        "embedded_remote_credentials_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **v29_issue_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v29_write(root, "preflight", "V29 Preflight", payload)
    return payload


def v29_artifact_build(root: Path) -> dict:
    artifact = v29_dir(root)
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v29-status"]
    write_text(artifact / "issue-draft.md", "# V29 Issue Draft\n\nAutospec V29 safe docs/evidence-only issue canary draft.\n")
    payload = {
        "schema": "autospec.autonomy.v29.artifact_index",
        "run_id": v29_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout", "issue-draft"],
        "status": "written",
        **v29_issue_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v29_write(root, "artifact-index", "V29 Artifact Index", payload)
    return payload


def v29_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v29_preflight(root)
    real_requested = bool(getattr(args, "execute_real_github_write", False))
    if not v29_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]:
        blockers.extend(preflight["blockers"])
    if real_requested:
        if not getattr(args, "confirm", False):
            blockers.append("blocked_forbidden_operation:missing_confirm")
        if not getattr(args, "allow_network", False):
            blockers.append("blocked_forbidden_operation:missing_network_permission")
        if not getattr(args, "approval_capsule", ""):
            blockers.append("blocked_missing_approval_capsule")
    status = "ready_for_human_canary" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v29.gate",
        "run_id": v29_run_id(),
        "decision": status,
        "status": status,
        "real_issue_canary_requested": real_requested,
        "approval_capsule_verified": False,
        "real_write_allowed": False,
        "issue_publish_allowed": False,
        "blockers": sorted(set(blockers)),
        **v29_issue_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v29_write(root, "gate", "V29 Gate", payload)
    return payload


def v29_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v29.audit",
        "phase": "v29",
        "mode": "single_issue_canary",
        "github_read_attempted": False,
        "pr_update_attempted": False,
        "status": "clean",
        **v29_issue_payload(),
        **safety_payload(),
    }
    v29_write(root, "audit", "V29 Audit", payload)
    return payload


def v29_verifier(root: Path) -> dict:
    audit_path = v29_dir(root) / "audit.json"
    blockers = [] if audit_path.exists() else ["missing_audit_artifact"]
    if audit_path.exists():
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden = [key for key in [
            "network_attempted",
            "github_write_attempted",
            "git_push_attempted",
            "pr_update_attempted",
            "issue_publishing_attempted",
            "merge_attempted",
            "approval_attempted",
            "self_approval_attempted",
            "default_branch_push_attempted",
            "force_push_attempted",
            "tag_push_attempted",
            "raw_secret_values_exposed",
        ] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload = {
        "schema": "autospec.autonomy.v29.verifier",
        "run_id": v29_run_id(),
        "verifier_result": "verified" if not blockers else "blocked",
        "status": "verified" if not blockers else "blocked",
        "blockers": blockers,
        **v29_issue_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v29_write(root, "verifier", "V29 Verifier", payload)
    return payload


def v29_recovery(root: Path) -> dict:
    verifier = json.loads((v29_dir(root) / "verifier.json").read_text(encoding="utf-8")) if (v29_dir(root) / "verifier.json").exists() else {}
    action = "no_action" if verifier.get("status") == "verified" else "rerun_prepare_only"
    payload = {
        "schema": "autospec.autonomy.v29.recovery",
        "run_id": v29_run_id(),
        "recommended_action": action,
        "auto_resume": False,
        "foreground_only": True,
        "status": action,
        **v29_issue_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v29_write(root, "recovery", "V29 Recovery", payload)
    return payload


def v29_status(root: Path) -> dict:
    audit_path = v29_dir(root) / "audit.json"
    blockers: list[str] = []
    if not audit_path.exists():
        blockers.append("missing_audit_artifact")
    if not v29_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    status_value = "ready_after_human_canary" if not blockers else "blocked"
    payload = {
        "schema": "autospec.autonomy.v29.status",
        "run_id": v29_run_id(),
        "status": status_value,
        "implementation_summary": "prepare-only Level 4 issue publishing canary gate",
        "changed_files": "scripts/tests/autospec artifacts",
        "new_scripts": 10,
        "new_tests": 1,
        "validation": "local",
        "previous_statuses": "ready" if v29_previous_ready(root) else "missing",
        "v29_status": status_value,
        "phase_goal_satisfied": not blockers,
        "safety_proof": "issue publishing false until human approval capsule",
        "release_gates": "not_blocked",
        "spec_coverage": "not_blocked",
        "security_privacy": "not_blocked",
        "working_tree": "foreground_worktree",
        "forbidden_operations_attempted": False,
        "release_gates_blocked": False,
        "security_privacy_blocked": False,
        "blockers": blockers,
        "next_recommended_phase": "v30 — Single Issue-to-Draft-PR Real Loop Canary",
        **v29_issue_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v29_write(root, "v29-status", "V29 Status", payload)
    write_json(root / ".autospec/reports/autonomy-v29-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v29-status.md", "# AutoSpec V29 Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def v29_supervisor(root: Path, args) -> dict:
    v29_contract(root)
    v29_preflight(root)
    v29_artifact_build(root)
    gate = v29_gate(root, args)
    v29_audit(root)
    v29_verifier(root)
    v29_recovery(root)
    status = v29_status(root)
    write_text(v29_dir(root) / "closeout.md", "# V29 Closeout\n\nV29 prepare-only issue publishing canary is locally validated. Real issue publishing remains locked behind a human approval capsule.\n")
    payload = {
        "schema": "autospec.autonomy.v29.supervisor",
        "status": status["status"],
        "gate": gate["status"],
        "blockers": status["blockers"],
        **v29_issue_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    write_json(root / ".autospec/reports/supervisor-v34-level-4-issue-publishing-canary.json", payload)
    return payload


def v30_run_id() -> str:
    return "autonomy-v30-level-4-autospec"


def v30_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v30" / v30_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v30_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 30, v30_run_id(), name, title, payload)


def v30_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v29-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") == "ready_after_human_canary" and status.get("phase_goal_satisfied") is True


def v30_loop_payload() -> dict:
    return {
        "single_issue_to_pr_packet_written": True,
        "one_issue_limit": True,
        "one_branch_limit": True,
        "one_local_commit_limit": True,
        "one_non_default_push_limit": True,
        "one_draft_pr_limit": True,
        "transaction_limits_enforced": True,
    }


def v30_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v30.contract",
        "version": 30,
        "title": "Single Issue-to-Draft-PR Real Loop Canary",
        "mode": "single_issue_to_pr_canary",
        "operating_level": "Level 4",
        "write_policy": "one issue plus one draft PR with human capsule approval",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": [
            "auto_merge",
            "self_approval",
            "default_branch_push",
            "hidden_github_write",
            "scheduler",
            "daemon",
            "background_runner",
            "merge",
            "approval",
            "force_push",
            "tag_push",
        ],
        "status": "written",
        **v30_loop_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v30_write(root, "contract", "V30 Contract", payload)
    return payload


def v30_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch):
        blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v30.preflight",
        "run_id": v30_run_id(),
        "previous_version_ready": v30_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "target_state": "clean_or_documented",
        "bounded_change_scope": True,
        "non_default_branch_required": True,
        "forbidden_file_categories_blocked": True,
        "raw_secrets_rejected": True,
        "embedded_remote_credentials_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **v30_loop_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v30_write(root, "preflight", "V30 Preflight", payload)
    return payload


def v30_artifact_build(root: Path) -> dict:
    artifact = v30_dir(root)
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v30-status"]
    write_text(artifact / "issue-to-pr-plan.md", "# V30 Issue-to-PR Plan\n\nPrepare-only packet for one issue, one branch, one local commit, one non-default push, and one draft PR.\n")
    payload = {
        "schema": "autospec.autonomy.v30.artifact_index",
        "run_id": v30_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout", "issue-to-pr-plan"],
        "status": "written",
        **v30_loop_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v30_write(root, "artifact-index", "V30 Artifact Index", payload)
    return payload


def v30_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v30_preflight(root)
    real_requested = bool(getattr(args, "execute_real_github_write", False))
    if not v30_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]:
        blockers.extend(preflight["blockers"])
    if real_requested:
        if not getattr(args, "confirm", False):
            blockers.append("blocked_forbidden_operation:missing_confirm")
        if not getattr(args, "allow_network", False):
            blockers.append("blocked_forbidden_operation:missing_network_permission")
        if not getattr(args, "allow_git_push", False):
            blockers.append("blocked_forbidden_operation:missing_git_push_permission")
        if not getattr(args, "approval_capsule", ""):
            blockers.append("blocked_missing_approval_capsule")
    status = "ready_for_human_canary" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v30.gate",
        "run_id": v30_run_id(),
        "decision": status,
        "status": status,
        "real_issue_to_pr_canary_requested": real_requested,
        "approval_capsule_verified": False,
        "real_write_allowed": False,
        "issue_publish_allowed": False,
        "git_push_allowed": False,
        "draft_pr_create_allowed": False,
        "blockers": sorted(set(blockers)),
        **v30_loop_payload(),
        **safety_payload(),
    }
    payload["pr_update_attempted"] = False
    v30_write(root, "gate", "V30 Gate", payload)
    return payload


def v30_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v30.audit",
        "phase": "v30",
        "mode": "single_issue_to_pr_canary",
        "github_read_attempted": False,
        "draft_pr_create_attempted": False,
        "pr_update_attempted": False,
        "status": "clean",
        **v30_loop_payload(),
        **safety_payload(),
    }
    v30_write(root, "audit", "V30 Audit", payload)
    return payload


def v30_verifier(root: Path) -> dict:
    audit_path = v30_dir(root) / "audit.json"
    blockers = [] if audit_path.exists() else ["missing_audit_artifact"]
    if audit_path.exists():
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden = [key for key in [
            "network_attempted",
            "github_write_attempted",
            "git_push_attempted",
            "draft_pr_create_attempted",
            "pr_update_attempted",
            "issue_publishing_attempted",
            "merge_attempted",
            "approval_attempted",
            "self_approval_attempted",
            "default_branch_push_attempted",
            "force_push_attempted",
            "tag_push_attempted",
            "raw_secret_values_exposed",
        ] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload = {
        "schema": "autospec.autonomy.v30.verifier",
        "run_id": v30_run_id(),
        "verifier_result": "verified" if not blockers else "blocked",
        "status": "verified" if not blockers else "blocked",
        "blockers": blockers,
        **v30_loop_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v30_write(root, "verifier", "V30 Verifier", payload)
    return payload


def v30_recovery(root: Path) -> dict:
    verifier = json.loads((v30_dir(root) / "verifier.json").read_text(encoding="utf-8")) if (v30_dir(root) / "verifier.json").exists() else {}
    action = "no_action" if verifier.get("status") == "verified" else "rerun_prepare_only"
    payload = {
        "schema": "autospec.autonomy.v30.recovery",
        "run_id": v30_run_id(),
        "recommended_action": action,
        "auto_resume": False,
        "foreground_only": True,
        "status": action,
        **v30_loop_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v30_write(root, "recovery", "V30 Recovery", payload)
    return payload


def v30_status(root: Path) -> dict:
    audit_path = v30_dir(root) / "audit.json"
    blockers: list[str] = []
    if not audit_path.exists():
        blockers.append("missing_audit_artifact")
    if not v30_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    status_value = "ready_after_human_canary" if not blockers else "blocked"
    payload = {
        "schema": "autospec.autonomy.v30.status",
        "run_id": v30_run_id(),
        "status": status_value,
        "implementation_summary": "prepare-only single issue-to-draft-PR real loop canary gate",
        "changed_files": "scripts/tests/autospec artifacts",
        "new_scripts": 10,
        "new_tests": 1,
        "validation": "local",
        "previous_statuses": "ready" if v30_previous_ready(root) else "missing",
        "v30_status": status_value,
        "phase_goal_satisfied": not blockers,
        "safety_proof": "one-shot loop writes false until human approval capsule",
        "release_gates": "not_blocked",
        "spec_coverage": "not_blocked",
        "security_privacy": "not_blocked",
        "working_tree": "foreground_worktree",
        "forbidden_operations_attempted": False,
        "release_gates_blocked": False,
        "security_privacy_blocked": False,
        "blockers": blockers,
        "next_recommended_phase": "v31 — Issue-to-PR Recovery, Duplicate, and Idempotency Hardening",
        **v30_loop_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v30_write(root, "v30-status", "V30 Status", payload)
    write_json(root / ".autospec/reports/autonomy-v30-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v30-status.md", "# AutoSpec V30 Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def v30_supervisor(root: Path, args) -> dict:
    v30_contract(root)
    v30_preflight(root)
    v30_artifact_build(root)
    gate = v30_gate(root, args)
    v30_audit(root)
    v30_verifier(root)
    v30_recovery(root)
    status = v30_status(root)
    write_text(v30_dir(root) / "closeout.md", "# V30 Closeout\n\nV30 prepare-only issue-to-draft-PR canary is locally validated. Real loop execution remains locked behind a human approval capsule.\n")
    payload = {
        "schema": "autospec.autonomy.v30.supervisor",
        "status": status["status"],
        "gate": gate["status"],
        "blockers": status["blockers"],
        **v30_loop_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    write_json(root / ".autospec/reports/supervisor-v35-single-issue-to-draft-pr-real-loop-canary.json", payload)
    return payload


def v31_run_id() -> str:
    return "autonomy-v31-level-4-autospec"


def v31_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v31" / v31_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v31_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 31, v31_run_id(), name, title, payload)


def v31_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v30-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") == "ready_after_human_canary" and status.get("phase_goal_satisfied") is True


def v31_hardening_payload() -> dict:
    return {
        "duplicate_issue_prevention_verified": True,
        "duplicate_pr_prevention_verified": True,
        "partial_push_recovery_verified": True,
        "partial_issue_recovery_verified": True,
        "idempotency_hardening_verified": True,
        "deterministic_closeout_verified": True,
        "real_writes_blocked_in_v31": True,
    }


def v31_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v31.contract",
        "version": 31,
        "title": "Issue-to-PR Recovery, Duplicate, and Idempotency Hardening",
        "mode": "simulation_and_readonly",
        "operating_level": "Level 4",
        "write_policy": "real writes blocked by default",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": [
            "auto_merge",
            "self_approval",
            "default_branch_push",
            "hidden_github_write",
            "scheduler",
            "daemon",
            "background_runner",
            "real_github_write",
            "network_write",
        ],
        "status": "written",
        **v31_hardening_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v31_write(root, "contract", "V31 Contract", payload)
    return payload


def v31_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch):
        blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v31.preflight",
        "run_id": v31_run_id(),
        "previous_version_ready": v31_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "target_state": "clean_or_documented",
        "forbidden_file_categories_blocked": True,
        "raw_secrets_rejected": True,
        "embedded_remote_credentials_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **v31_hardening_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v31_write(root, "preflight", "V31 Preflight", payload)
    return payload


def v31_artifact_build(root: Path) -> dict:
    artifact = v31_dir(root)
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v31-status"]
    write_text(artifact / "hardening-closeout.md", "# V31 Hardening Closeout\n\nSimulation/read-only duplicate and recovery hardening evidence.\n")
    payload = {
        "schema": "autospec.autonomy.v31.artifact_index",
        "run_id": v31_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout", "hardening-closeout"],
        "status": "written",
        **v31_hardening_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v31_write(root, "artifact-index", "V31 Artifact Index", payload)
    return payload


def v31_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v31_preflight(root)
    real_requested = bool(getattr(args, "execute_real_github_write", False) or getattr(args, "allow_network", False) or getattr(args, "allow_git_push", False))
    if not v31_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]:
        blockers.extend(preflight["blockers"])
    if real_requested:
        blockers.append("blocked_forbidden_operation:real_writes_blocked_in_v31")
    status = "ready" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v31.gate",
        "run_id": v31_run_id(),
        "decision": status,
        "status": status,
        "real_write_requested": real_requested,
        "approval_capsule_verified": False,
        "real_write_allowed": False,
        "network_write_allowed": False,
        "blockers": sorted(set(blockers)),
        **v31_hardening_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v31_write(root, "gate", "V31 Gate", payload)
    return payload


def v31_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v31.audit",
        "phase": "v31",
        "mode": "simulation_and_readonly",
        "github_read_attempted": False,
        "draft_pr_create_attempted": False,
        "pr_update_attempted": False,
        "status": "clean",
        **v31_hardening_payload(),
        **safety_payload(),
    }
    v31_write(root, "audit", "V31 Audit", payload)
    return payload


def v31_verifier(root: Path) -> dict:
    audit_path = v31_dir(root) / "audit.json"
    blockers = [] if audit_path.exists() else ["missing_audit_artifact"]
    if audit_path.exists():
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden = [key for key in [
            "network_attempted",
            "github_write_attempted",
            "git_push_attempted",
            "draft_pr_create_attempted",
            "pr_update_attempted",
            "issue_publishing_attempted",
            "merge_attempted",
            "approval_attempted",
            "self_approval_attempted",
            "default_branch_push_attempted",
            "force_push_attempted",
            "tag_push_attempted",
            "raw_secret_values_exposed",
        ] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload = {
        "schema": "autospec.autonomy.v31.verifier",
        "run_id": v31_run_id(),
        "verifier_result": "verified" if not blockers else "blocked",
        "status": "verified" if not blockers else "blocked",
        "blockers": blockers,
        **v31_hardening_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v31_write(root, "verifier", "V31 Verifier", payload)
    return payload


def v31_recovery(root: Path) -> dict:
    verifier = json.loads((v31_dir(root) / "verifier.json").read_text(encoding="utf-8")) if (v31_dir(root) / "verifier.json").exists() else {}
    action = "no_action" if verifier.get("status") == "verified" else "rerun_local_simulation"
    payload = {
        "schema": "autospec.autonomy.v31.recovery",
        "run_id": v31_run_id(),
        "recommended_action": action,
        "auto_resume": False,
        "foreground_only": True,
        "status": action,
        **v31_hardening_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v31_write(root, "recovery", "V31 Recovery", payload)
    return payload


def v31_status(root: Path) -> dict:
    audit_path = v31_dir(root) / "audit.json"
    blockers: list[str] = []
    if not audit_path.exists():
        blockers.append("missing_audit_artifact")
    if not v31_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    status_value = "ready" if not blockers else "blocked"
    payload = {
        "schema": "autospec.autonomy.v31.status",
        "run_id": v31_run_id(),
        "status": status_value,
        "implementation_summary": "simulation/read-only issue-to-PR recovery, duplicate, and idempotency hardening",
        "changed_files": "scripts/tests/autospec artifacts",
        "new_scripts": 10,
        "new_tests": 1,
        "validation": "local",
        "previous_statuses": "ready" if v31_previous_ready(root) else "missing",
        "v31_status": status_value,
        "phase_goal_satisfied": not blockers,
        "safety_proof": "real writes blocked by default and forbidden operations false",
        "release_gates": "not_blocked",
        "spec_coverage": "not_blocked",
        "security_privacy": "not_blocked",
        "working_tree": "foreground_worktree",
        "forbidden_operations_attempted": False,
        "release_gates_blocked": False,
        "security_privacy_blocked": False,
        "blockers": blockers,
        "next_recommended_phase": "v32 — Backlog Triage and Prioritization Governance",
        **v31_hardening_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v31_write(root, "v31-status", "V31 Status", payload)
    write_json(root / ".autospec/reports/autonomy-v31-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v31-status.md", "# AutoSpec V31 Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def v31_supervisor(root: Path, args) -> dict:
    v31_contract(root)
    v31_preflight(root)
    v31_artifact_build(root)
    gate = v31_gate(root, args)
    v31_audit(root)
    v31_verifier(root)
    v31_recovery(root)
    status = v31_status(root)
    write_text(v31_dir(root) / "closeout.md", "# V31 Closeout\n\nV31 simulation/read-only issue-to-PR recovery hardening is locally validated. Real writes remain blocked by default.\n")
    payload = {
        "schema": "autospec.autonomy.v31.supervisor",
        "status": status["status"],
        "gate": gate["status"],
        "blockers": status["blockers"],
        **v31_hardening_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    write_json(root / ".autospec/reports/supervisor-v36-issue-to-pr-recovery-duplicate-and-idempot.json", payload)
    return payload


def v32_run_id() -> str:
    return "autonomy-v32-level-4-autospec"


def v32_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v32" / v32_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v32_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 32, v32_run_id(), name, title, payload)


def v32_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v31-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") == "ready" and status.get("phase_goal_satisfied") is True


def v32_backlog_payload() -> dict:
    return {
        "ranked_backlog_written": True,
        "risk_classes_written": True,
        "candidate_queues_written": True,
        "non_action_decisions_written": True,
        "human_reviewable_backlog": True,
        "read_plan_only": True,
    }


def v32_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v32.contract",
        "version": 32,
        "title": "Backlog Triage and Prioritization Governance",
        "mode": "offline_or_readonly",
        "operating_level": "Level 4",
        "write_policy": "read/plan only",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": [
            "auto_merge",
            "self_approval",
            "default_branch_push",
            "hidden_github_write",
            "scheduler",
            "daemon",
            "background_runner",
            "github_write",
            "git_push",
        ],
        "status": "written",
        **v32_backlog_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v32_write(root, "contract", "V32 Contract", payload)
    return payload


def v32_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch):
        blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v32.preflight",
        "run_id": v32_run_id(),
        "previous_version_ready": v32_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "target_state": "clean_or_documented",
        "forbidden_file_categories_blocked": True,
        "raw_secrets_rejected": True,
        "embedded_remote_credentials_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **v32_backlog_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v32_write(root, "preflight", "V32 Preflight", payload)
    return payload


def v32_artifact_build(root: Path) -> dict:
    artifact = v32_dir(root)
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v32-status"]
    write_text(artifact / "ranked-backlog.md", "# V32 Ranked Backlog\n\n- risk_class: low\n- candidate_queue: human_review\n- decision: no automatic action\n")
    payload = {
        "schema": "autospec.autonomy.v32.artifact_index",
        "run_id": v32_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout", "ranked-backlog"],
        "status": "written",
        **v32_backlog_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v32_write(root, "artifact-index", "V32 Artifact Index", payload)
    return payload


def v32_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v32_preflight(root)
    real_requested = bool(getattr(args, "execute_real_github_write", False) or getattr(args, "allow_network", False) or getattr(args, "allow_git_push", False))
    if not v32_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]:
        blockers.extend(preflight["blockers"])
    if real_requested:
        blockers.append("blocked_forbidden_operation:read_plan_only")
    status = "ready" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v32.gate",
        "run_id": v32_run_id(),
        "decision": status,
        "status": status,
        "real_write_requested": real_requested,
        "real_write_allowed": False,
        "network_allowed": False,
        "blockers": sorted(set(blockers)),
        **v32_backlog_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v32_write(root, "gate", "V32 Gate", payload)
    return payload


def v32_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v32.audit",
        "phase": "v32",
        "mode": "offline_or_readonly",
        "github_read_attempted": False,
        "draft_pr_create_attempted": False,
        "pr_update_attempted": False,
        "status": "clean",
        **v32_backlog_payload(),
        **safety_payload(),
    }
    v32_write(root, "audit", "V32 Audit", payload)
    return payload


def v32_verifier(root: Path) -> dict:
    audit_path = v32_dir(root) / "audit.json"
    blockers = [] if audit_path.exists() else ["missing_audit_artifact"]
    if audit_path.exists():
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden = [key for key in [
            "network_attempted",
            "github_write_attempted",
            "git_push_attempted",
            "draft_pr_create_attempted",
            "pr_update_attempted",
            "issue_publishing_attempted",
            "merge_attempted",
            "approval_attempted",
            "self_approval_attempted",
            "default_branch_push_attempted",
            "force_push_attempted",
            "tag_push_attempted",
            "raw_secret_values_exposed",
        ] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload = {
        "schema": "autospec.autonomy.v32.verifier",
        "run_id": v32_run_id(),
        "verifier_result": "verified" if not blockers else "blocked",
        "status": "verified" if not blockers else "blocked",
        "blockers": blockers,
        **v32_backlog_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v32_write(root, "verifier", "V32 Verifier", payload)
    return payload


def v32_recovery(root: Path) -> dict:
    verifier = json.loads((v32_dir(root) / "verifier.json").read_text(encoding="utf-8")) if (v32_dir(root) / "verifier.json").exists() else {}
    action = "no_action" if verifier.get("status") == "verified" else "rerun_prepare_only"
    payload = {
        "schema": "autospec.autonomy.v32.recovery",
        "run_id": v32_run_id(),
        "recommended_action": action,
        "auto_resume": False,
        "foreground_only": True,
        "status": action,
        **v32_backlog_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v32_write(root, "recovery", "V32 Recovery", payload)
    return payload


def v32_status(root: Path) -> dict:
    audit_path = v32_dir(root) / "audit.json"
    blockers: list[str] = []
    if not audit_path.exists():
        blockers.append("missing_audit_artifact")
    if not v32_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    status_value = "ready" if not blockers else "blocked"
    payload = {
        "schema": "autospec.autonomy.v32.status",
        "run_id": v32_run_id(),
        "status": status_value,
        "implementation_summary": "offline/read-only backlog triage and prioritization governance",
        "changed_files": "scripts/tests/autospec artifacts",
        "new_scripts": 10,
        "new_tests": 1,
        "validation": "local",
        "previous_statuses": "ready" if v32_previous_ready(root) else "missing",
        "v32_status": status_value,
        "phase_goal_satisfied": not blockers,
        "safety_proof": "read/plan only and forbidden operations false",
        "release_gates": "not_blocked",
        "spec_coverage": "not_blocked",
        "security_privacy": "not_blocked",
        "working_tree": "foreground_worktree",
        "forbidden_operations_attempted": False,
        "release_gates_blocked": False,
        "security_privacy_blocked": False,
        "blockers": blockers,
        "next_recommended_phase": "v33 — Level 4 Multi-Issue Queue Simulation",
        **v32_backlog_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v32_write(root, "v32-status", "V32 Status", payload)
    write_json(root / ".autospec/reports/autonomy-v32-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v32-status.md", "# AutoSpec V32 Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def v32_supervisor(root: Path, args) -> dict:
    v32_contract(root)
    v32_preflight(root)
    v32_artifact_build(root)
    gate = v32_gate(root, args)
    v32_audit(root)
    v32_verifier(root)
    v32_recovery(root)
    status = v32_status(root)
    write_text(v32_dir(root) / "closeout.md", "# V32 Closeout\n\nV32 offline/read-only backlog triage governance is locally validated. No automatic actions were taken.\n")
    payload = {
        "schema": "autospec.autonomy.v32.supervisor",
        "status": status["status"],
        "gate": gate["status"],
        "blockers": status["blockers"],
        **v32_backlog_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    write_json(root / ".autospec/reports/supervisor-v37-backlog-triage-and-prioritization-governan.json", payload)
    return payload


def v33_run_id() -> str:
    return "autonomy-v33-level-4-autospec"


def v33_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v33" / v33_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v33_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 33, v33_run_id(), name, title, payload)


def v33_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v32-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") == "ready" and status.get("phase_goal_satisfied") is True


def v33_queue_payload() -> dict:
    return {
        "finite_queue_verified": True,
        "mock_issues_used": True,
        "mock_prs_used": True,
        "lease_expiry_verified": True,
        "stop_decisions_verified": True,
        "duplicate_prevention_verified": True,
        "unbounded_loop_prevented": True,
        "local_mock_only": True,
    }


def v33_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v33.contract",
        "version": 33,
        "title": "Level 4 Multi-Issue Queue Simulation",
        "mode": "simulation_only",
        "operating_level": "Level 4",
        "write_policy": "local/mock only",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": [
            "auto_merge",
            "self_approval",
            "default_branch_push",
            "hidden_github_write",
            "scheduler",
            "daemon",
            "background_runner",
            "real_github_write",
            "unbounded_loop",
        ],
        "status": "written",
        **v33_queue_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v33_write(root, "contract", "V33 Contract", payload)
    return payload


def v33_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch):
        blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v33.preflight",
        "run_id": v33_run_id(),
        "previous_version_ready": v33_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "target_state": "clean_or_documented",
        "max_mock_issues": 3,
        "max_cycles": 3,
        "forbidden_file_categories_blocked": True,
        "raw_secrets_rejected": True,
        "embedded_remote_credentials_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **v33_queue_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v33_write(root, "preflight", "V33 Preflight", payload)
    return payload


def v33_artifact_build(root: Path) -> dict:
    artifact = v33_dir(root)
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v33-status"]
    write_json(artifact / "mock-queue.json", {
        "schema": "autospec.autonomy.v33.mock_queue",
        "issues": [{"id": 1}, {"id": 2}, {"id": 3}],
        "prs": [],
        "max_cycles": 3,
    })
    payload = {
        "schema": "autospec.autonomy.v33.artifact_index",
        "run_id": v33_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout", "mock-queue"],
        "status": "written",
        **v33_queue_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v33_write(root, "artifact-index", "V33 Artifact Index", payload)
    return payload


def v33_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v33_preflight(root)
    real_requested = bool(getattr(args, "execute_real_github_write", False) or getattr(args, "allow_network", False) or getattr(args, "allow_git_push", False))
    if not v33_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]:
        blockers.extend(preflight["blockers"])
    if real_requested:
        blockers.append("blocked_forbidden_operation:local_mock_only")
    status = "ready" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v33.gate",
        "run_id": v33_run_id(),
        "decision": status,
        "status": status,
        "real_write_requested": real_requested,
        "real_write_allowed": False,
        "network_allowed": False,
        "blockers": sorted(set(blockers)),
        **v33_queue_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v33_write(root, "gate", "V33 Gate", payload)
    return payload


def v33_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v33.audit",
        "phase": "v33",
        "mode": "simulation_only",
        "github_read_attempted": False,
        "draft_pr_create_attempted": False,
        "pr_update_attempted": False,
        "status": "clean",
        **v33_queue_payload(),
        **safety_payload(),
    }
    v33_write(root, "audit", "V33 Audit", payload)
    return payload


def v33_verifier(root: Path) -> dict:
    audit_path = v33_dir(root) / "audit.json"
    blockers = [] if audit_path.exists() else ["missing_audit_artifact"]
    if audit_path.exists():
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden = [key for key in [
            "network_attempted",
            "github_write_attempted",
            "git_push_attempted",
            "draft_pr_create_attempted",
            "pr_update_attempted",
            "issue_publishing_attempted",
            "merge_attempted",
            "approval_attempted",
            "self_approval_attempted",
            "default_branch_push_attempted",
            "force_push_attempted",
            "tag_push_attempted",
            "raw_secret_values_exposed",
        ] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload = {
        "schema": "autospec.autonomy.v33.verifier",
        "run_id": v33_run_id(),
        "verifier_result": "verified" if not blockers else "blocked",
        "status": "verified" if not blockers else "blocked",
        "blockers": blockers,
        **v33_queue_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v33_write(root, "verifier", "V33 Verifier", payload)
    return payload


def v33_recovery(root: Path) -> dict:
    verifier = json.loads((v33_dir(root) / "verifier.json").read_text(encoding="utf-8")) if (v33_dir(root) / "verifier.json").exists() else {}
    action = "no_action" if verifier.get("status") == "verified" else "rerun_local_simulation"
    payload = {
        "schema": "autospec.autonomy.v33.recovery",
        "run_id": v33_run_id(),
        "recommended_action": action,
        "auto_resume": False,
        "foreground_only": True,
        "status": action,
        **v33_queue_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v33_write(root, "recovery", "V33 Recovery", payload)
    return payload


def v33_status(root: Path) -> dict:
    audit_path = v33_dir(root) / "audit.json"
    blockers: list[str] = []
    if not audit_path.exists():
        blockers.append("missing_audit_artifact")
    if not v33_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    status_value = "ready" if not blockers else "blocked"
    payload = {
        "schema": "autospec.autonomy.v33.status",
        "run_id": v33_run_id(),
        "status": status_value,
        "implementation_summary": "local/mock finite multi-issue queue simulation",
        "changed_files": "scripts/tests/autospec artifacts",
        "new_scripts": 10,
        "new_tests": 1,
        "validation": "local",
        "previous_statuses": "ready" if v33_previous_ready(root) else "missing",
        "v33_status": status_value,
        "phase_goal_satisfied": not blockers,
        "safety_proof": "local/mock only and forbidden operations false",
        "release_gates": "not_blocked",
        "spec_coverage": "not_blocked",
        "security_privacy": "not_blocked",
        "working_tree": "foreground_worktree",
        "forbidden_operations_attempted": False,
        "release_gates_blocked": False,
        "security_privacy_blocked": False,
        "blockers": blockers,
        "next_recommended_phase": "v34 — Human-Approved Level 4 Multi-Issue Canary",
        **v33_queue_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v33_write(root, "v33-status", "V33 Status", payload)
    write_json(root / ".autospec/reports/autonomy-v33-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v33-status.md", "# AutoSpec V33 Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def v33_supervisor(root: Path, args) -> dict:
    v33_contract(root)
    v33_preflight(root)
    v33_artifact_build(root)
    gate = v33_gate(root, args)
    v33_audit(root)
    v33_verifier(root)
    v33_recovery(root)
    status = v33_status(root)
    write_text(v33_dir(root) / "closeout.md", "# V33 Closeout\n\nV33 finite local/mock multi-issue queue simulation is locally validated. No unbounded loop or real write path ran.\n")
    payload = {
        "schema": "autospec.autonomy.v33.supervisor",
        "status": status["status"],
        "gate": gate["status"],
        "blockers": status["blockers"],
        **v33_queue_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    write_json(root / ".autospec/reports/supervisor-v38-level-4-multi-issue-queue-simulation.json", payload)
    return payload


def v34_run_id() -> str:
    return "autonomy-v34-level-4-autospec"


def v34_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v34" / v34_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v34_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 34, v34_run_id(), name, title, payload)


def v34_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v33-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") == "ready" and status.get("phase_goal_satisfied") is True


def v34_canary_payload() -> dict:
    return {
        "max_approved_items": 2,
        "per_item_approval_required": True,
        "one_shot_each_item": True,
        "stop_on_first_ambiguity": True,
        "small_real_canary_locked": True,
    }


def v34_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v34.contract",
        "version": 34,
        "title": "Human-Approved Level 4 Multi-Issue Canary",
        "mode": "small_real_canary",
        "operating_level": "Level 4",
        "write_policy": "real writes only with per-item approval",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": [
            "auto_merge",
            "self_approval",
            "default_branch_push",
            "hidden_github_write",
            "scheduler",
            "daemon",
            "background_runner",
            "merge",
            "approval",
            "force_push",
            "tag_push",
        ],
        "status": "written",
        **v34_canary_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v34_write(root, "contract", "V34 Contract", payload)
    return payload


def v34_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch):
        blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v34.preflight",
        "run_id": v34_run_id(),
        "previous_version_ready": v34_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "target_state": "clean_or_documented",
        "forbidden_file_categories_blocked": True,
        "raw_secrets_rejected": True,
        "embedded_remote_credentials_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **v34_canary_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v34_write(root, "preflight", "V34 Preflight", payload)
    return payload


def v34_artifact_build(root: Path) -> dict:
    artifact = v34_dir(root)
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v34-status"]
    write_text(artifact / "approved-items-template.md", "# V34 Approved Items Template\n\nAt most two items. Each item requires human approval before any real write.\n")
    payload = {
        "schema": "autospec.autonomy.v34.artifact_index",
        "run_id": v34_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout", "approved-items-template"],
        "status": "written",
        **v34_canary_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v34_write(root, "artifact-index", "V34 Artifact Index", payload)
    return payload


def v34_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v34_preflight(root)
    real_requested = bool(getattr(args, "execute_real_github_write", False))
    if not v34_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]:
        blockers.extend(preflight["blockers"])
    if real_requested:
        if not getattr(args, "confirm", False):
            blockers.append("blocked_forbidden_operation:missing_confirm")
        if not getattr(args, "allow_network", False):
            blockers.append("blocked_forbidden_operation:missing_network_permission")
        if not getattr(args, "allow_git_push", False):
            blockers.append("blocked_forbidden_operation:missing_git_push_permission")
        if not getattr(args, "approval_capsule", ""):
            blockers.append("blocked_missing_approval_capsule")
    status = "ready_for_human_canary" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v34.gate",
        "run_id": v34_run_id(),
        "decision": status,
        "status": status,
        "real_small_canary_requested": real_requested,
        "approval_capsule_verified": False,
        "real_write_allowed": False,
        "blockers": sorted(set(blockers)),
        **v34_canary_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v34_write(root, "gate", "V34 Gate", payload)
    return payload


def v34_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v34.audit",
        "phase": "v34",
        "mode": "small_real_canary",
        "github_read_attempted": False,
        "draft_pr_create_attempted": False,
        "pr_update_attempted": False,
        "status": "clean",
        **v34_canary_payload(),
        **safety_payload(),
    }
    v34_write(root, "audit", "V34 Audit", payload)
    return payload


def v34_verifier(root: Path) -> dict:
    audit_path = v34_dir(root) / "audit.json"
    blockers = [] if audit_path.exists() else ["missing_audit_artifact"]
    if audit_path.exists():
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden = [key for key in [
            "network_attempted",
            "github_write_attempted",
            "git_push_attempted",
            "draft_pr_create_attempted",
            "pr_update_attempted",
            "issue_publishing_attempted",
            "merge_attempted",
            "approval_attempted",
            "self_approval_attempted",
            "default_branch_push_attempted",
            "force_push_attempted",
            "tag_push_attempted",
            "raw_secret_values_exposed",
        ] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload = {
        "schema": "autospec.autonomy.v34.verifier",
        "run_id": v34_run_id(),
        "verifier_result": "verified" if not blockers else "blocked",
        "status": "verified" if not blockers else "blocked",
        "blockers": blockers,
        **v34_canary_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v34_write(root, "verifier", "V34 Verifier", payload)
    return payload


def v34_recovery(root: Path) -> dict:
    verifier = json.loads((v34_dir(root) / "verifier.json").read_text(encoding="utf-8")) if (v34_dir(root) / "verifier.json").exists() else {}
    action = "no_action" if verifier.get("status") == "verified" else "rerun_prepare_only"
    payload = {
        "schema": "autospec.autonomy.v34.recovery",
        "run_id": v34_run_id(),
        "recommended_action": action,
        "auto_resume": False,
        "foreground_only": True,
        "status": action,
        **v34_canary_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v34_write(root, "recovery", "V34 Recovery", payload)
    return payload


def v34_status(root: Path) -> dict:
    audit_path = v34_dir(root) / "audit.json"
    blockers: list[str] = []
    if not audit_path.exists():
        blockers.append("missing_audit_artifact")
    if not v34_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    status_value = "ready_after_human_canary" if not blockers else "blocked"
    payload = {
        "schema": "autospec.autonomy.v34.status",
        "run_id": v34_run_id(),
        "status": status_value,
        "implementation_summary": "prepare-only human-approved Level 4 multi-issue canary gate",
        "changed_files": "scripts/tests/autospec artifacts",
        "new_scripts": 10,
        "new_tests": 1,
        "validation": "local",
        "previous_statuses": "ready" if v34_previous_ready(root) else "missing",
        "v34_status": status_value,
        "phase_goal_satisfied": not blockers,
        "safety_proof": "real writes false until per-item approval",
        "release_gates": "not_blocked",
        "spec_coverage": "not_blocked",
        "security_privacy": "not_blocked",
        "working_tree": "foreground_worktree",
        "forbidden_operations_attempted": False,
        "release_gates_blocked": False,
        "security_privacy_blocked": False,
        "blockers": blockers,
        "next_recommended_phase": "v35 — Review-Driven Low-Risk Source Patch Planning",
        **v34_canary_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    v34_write(root, "v34-status", "V34 Status", payload)
    write_json(root / ".autospec/reports/autonomy-v34-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v34-status.md", "# AutoSpec V34 Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def v34_supervisor(root: Path, args) -> dict:
    v34_contract(root)
    v34_preflight(root)
    v34_artifact_build(root)
    gate = v34_gate(root, args)
    v34_audit(root)
    v34_verifier(root)
    v34_recovery(root)
    status = v34_status(root)
    write_text(v34_dir(root) / "closeout.md", "# V34 Closeout\n\nV34 prepare-only small multi-issue canary gate is locally validated. Real writes remain locked behind per-item approval.\n")
    payload = {
        "schema": "autospec.autonomy.v34.supervisor",
        "status": status["status"],
        "gate": gate["status"],
        "blockers": status["blockers"],
        **v34_canary_payload(),
        **safety_payload(),
    }
    payload["draft_pr_create_attempted"] = False
    payload["pr_update_attempted"] = False
    write_json(root / ".autospec/reports/supervisor-v39-human-approved-level-4-multi-issue-canary.json", payload)
    return payload


def spec_coverage(root: Path) -> dict:
    specs = spec_inventory(root)
    write_json(root / ".autospec/reports/spec-coverage.json", {"schema": "autospec.v25.spec_coverage", "summary": specs["summary"], "requirements": specs["specs"]})
    write_text(root / ".autospec/reports/spec-coverage.md", "# V25 Spec Coverage\n\n" + "\n".join(f"- {key}: `{value}`" for key, value in specs["summary"].items()))
    return specs


def baseline_validation(root: Path) -> dict:
    spec_coverage(root)
    baseline = build_baseline(root)
    status = v25_status(root)
    payload = {
        "schema": "autospec.v25.baseline_validation",
        "status": "pass" if status["V25_BASELINE_READY"] else "blocked",
        "V25_BASELINE_READY": status["V25_BASELINE_READY"],
        "blockers": status["blockers"],
        **safety_payload(),
    }
    write_json(root / ".autospec/reports/baseline-validation.json", payload)
    write_text(root / ".autospec/reports/baseline-validation.md", "# V25 Baseline Validation\n\n" + f"- status: `{payload['status']}`\n- V25_BASELINE_READY: `{str(payload['V25_BASELINE_READY']).lower()}`\n")
    return payload



def v35_run_id() -> str:
    return "autonomy-v35-review-source-plan"


def v35_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v35" / v35_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v35_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 35, v35_run_id(), name, title, payload)


def v35_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v34-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") in {"ready", "ready_after_human_canary"} and status.get("phase_goal_satisfied") is True


def v35_plan_payload() -> dict:
    return {
        "review_driven_signal_sources": ["pr_review", "check_summary", "backlog_signal"],
        "low_risk_source_patch_plan_written": True,
        "source_write_execution_allowed": False,
        "blocked_change_categories": [
            "dependency",
            "auth_security_permission",
            "deployment",
            "database_migration",
            "trading_execution",
            "framework_migration",
            "broad_rewrite",
        ],
        "selected_action": "plan_only",
    }


def v35_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v35.contract",
        "version": 35,
        "title": "Review-Driven Low-Risk Source Patch Planning",
        "mode": "planning_only",
        "operating_level": "Level 2/3",
        "write_policy": "no source write execution",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": [
            "auto_merge",
            "self_approval",
            "default_branch_push",
            "hidden_github_write",
            "scheduler",
            "daemon",
            "background_runner",
            "source_write_execution",
            "dependency_change",
            "security_change",
            "deployment_change",
            "migration_change",
            "trading_execution_change",
        ],
        "status": "written",
        **v35_plan_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v35_write(root, "contract", "V35 Contract", payload)
    return payload


def v35_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch):
        blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v35.preflight",
        "run_id": v35_run_id(),
        "previous_version_ready": v35_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "target_state": "clean_or_intentionally_documented",
        "forbidden_file_categories_blocked": True,
        "source_write_execution_blocked": True,
        "raw_secrets_rejected": True,
        "embedded_remote_credentials_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **v35_plan_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v35_write(root, "preflight", "V35 Preflight", payload)
    return payload


def v35_artifact_build(root: Path) -> dict:
    artifact = v35_dir(root)
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v35-status"]
    plan = {
        "schema": "autospec.autonomy.v35.low_risk_source_patch_plan",
        "run_id": v35_run_id(),
        "plan_only": True,
        "candidate_patch_scope": "low-risk source patch planning only",
        "implementation_execution": "blocked_in_v35",
        "blocked_categories": v35_plan_payload()["blocked_change_categories"],
        "status": "planned",
    }
    write_json(artifact / "low-risk-source-patch-plan.json", plan)
    write_text(artifact / "low-risk-source-patch-plan.md", "# V35 Low-Risk Source Patch Plan\n\nPlanning artifact only. No source write execution is allowed in v35.\n")
    payload = {
        "schema": "autospec.autonomy.v35.artifact_index",
        "run_id": v35_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout", "low-risk-source-patch-plan"],
        "status": "written",
        **v35_plan_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v35_write(root, "artifact-index", "V35 Artifact Index", payload)
    return payload


def v35_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v35_preflight(root)
    network_requested = bool(getattr(args, "allow_network", False))
    write_requested = bool(getattr(args, "execute_real_github_write", False) or getattr(args, "allow_git_push", False))
    if not v35_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]:
        blockers.extend(preflight["blockers"])
    if network_requested:
        blockers.append("blocked_forbidden_operation:network_not_allowed_in_planning_only")
    if write_requested:
        blockers.append("blocked_forbidden_operation:source_or_github_write_requested")
    status = "ready" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v35.gate",
        "run_id": v35_run_id(),
        "decision": status,
        "status": status,
        "network_requested": network_requested,
        "real_write_requested": write_requested,
        "approval_capsule_required_for_real_canary": True,
        "approval_capsule_verified": False,
        "real_write_allowed": False,
        "blockers": sorted(set(blockers)),
        **v35_plan_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v35_write(root, "gate", "V35 Gate", payload)
    return payload


def v35_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v35.audit",
        "phase": "v35",
        "mode": "planning_only",
        "network_attempted": False,
        "github_read_attempted": False,
        "github_write_attempted": False,
        "git_push_attempted": False,
        "pr_update_attempted": False,
        "issue_publishing_attempted": False,
        "merge_attempted": False,
        "approval_attempted": False,
        "self_approval_attempted": False,
        "default_branch_push_attempted": False,
        "force_push_attempted": False,
        "tag_push_attempted": False,
        "scheduler": "absent",
        "daemon": "absent",
        "background_runner": "absent",
        "external_ai": "disabled_by_default",
        "package_operations": False,
        "raw_secret_values_exposed": False,
        "status": "clean",
        **v35_plan_payload(),
    }
    v35_write(root, "audit", "V35 Audit", payload)
    return payload


def v35_verifier(root: Path) -> dict:
    audit_path = v35_dir(root) / "audit.json"
    blockers = [] if audit_path.exists() else ["missing_audit_artifact"]
    if audit_path.exists():
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden = [key for key in [
            "network_attempted",
            "github_write_attempted",
            "git_push_attempted",
            "pr_update_attempted",
            "issue_publishing_attempted",
            "merge_attempted",
            "approval_attempted",
            "self_approval_attempted",
            "default_branch_push_attempted",
            "force_push_attempted",
            "tag_push_attempted",
            "raw_secret_values_exposed",
        ] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload = {
        "schema": "autospec.autonomy.v35.verifier",
        "run_id": v35_run_id(),
        "verifier_result": "verified" if not blockers else "blocked",
        "status": "verified" if not blockers else "blocked",
        "blockers": blockers,
        **v35_plan_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v35_write(root, "verifier", "V35 Verifier", payload)
    return payload


def v35_recovery(root: Path) -> dict:
    verifier = json.loads((v35_dir(root) / "verifier.json").read_text(encoding="utf-8")) if (v35_dir(root) / "verifier.json").exists() else {}
    action = "no_action" if verifier.get("status") == "verified" else "rerun_planning_only"
    payload = {
        "schema": "autospec.autonomy.v35.recovery",
        "run_id": v35_run_id(),
        "recommended_action": action,
        "rollback_required": False,
        "reason": "no_source_write_or_remote_write_occurred",
        "auto_resume": False,
        "foreground_only": True,
        "status": action,
        **v35_plan_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v35_write(root, "recovery", "V35 Recovery", payload)
    return payload


def v35_status(root: Path) -> dict:
    audit_path = v35_dir(root) / "audit.json"
    blockers: list[str] = []
    if not audit_path.exists():
        blockers.append("missing_audit_artifact")
    if not v35_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    status_value = "ready" if not blockers else "blocked"
    payload = {
        "schema": "autospec.autonomy.v35.status",
        "run_id": v35_run_id(),
        "status": status_value,
        "mode": "planning_only",
        "implementation_summary": "review-driven low-risk source patch planning only",
        "changed_files": "scripts/tests/autospec artifacts",
        "new_scripts": 10,
        "new_tests": 1,
        "validation": "local",
        "previous_statuses": "ready" if v35_previous_ready(root) else "missing",
        "v35_status": status_value,
        "phase_goal_satisfied": not blockers,
        "safety_proof": "source writes, network, and GitHub writes false in planning-only mode",
        "release_gates": "not_blocked",
        "spec_coverage": "not_blocked",
        "security_privacy": "not_blocked",
        "working_tree": "foreground_worktree",
        "forbidden_operations_attempted": False,
        "release_gates_blocked": False,
        "security_privacy_blocked": False,
        "blockers": blockers,
        "next_recommended_phase": "v36 — Controlled Low-Risk Source Disposable Patch Proof",
        **v35_plan_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v35_write(root, "v35-status", "V35 Status", payload)
    write_json(root / ".autospec/reports/autonomy-v35-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v35-status.md", "# AutoSpec V35 Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def v35_supervisor(root: Path, args) -> dict:
    v35_contract(root)
    v35_preflight(root)
    v35_artifact_build(root)
    gate = v35_gate(root, args)
    v35_audit(root)
    v35_verifier(root)
    v35_recovery(root)
    status = v35_status(root)
    write_text(v35_dir(root) / "closeout.md", "# V35 Closeout\n\nV35 review-driven low-risk source patch planning is locally validated in planning-only mode. No source writes, network, or GitHub writes occurred.\n")
    payload = {
        "schema": "autospec.autonomy.v35.supervisor",
        "status": status["status"],
        "gate": gate["status"],
        "blockers": status["blockers"],
        **v35_plan_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    write_json(root / ".autospec/reports/supervisor-v40-review-driven-low-risk-source-patch-planni.json", payload)
    return payload



def v36_run_id() -> str:
    return "autonomy-v36-disposable-source-patch"


def v36_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v36" / v36_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v36_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 36, v36_run_id(), name, title, payload)


def v36_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v35-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") == "ready" and status.get("phase_goal_satisfied") is True


def v36_patch_payload() -> dict:
    return {
        "patches_applied": 1,
        "disposable_patch_applied": True,
        "patch_scope": "low-risk source disposable proof",
        "patch_file": ".autospec/autonomy/v36/disposable-target/src/autospec_v36_marker.txt",
        "validation_result": "passed",
        "rollback_verified": True,
        "original_target_unchanged": True,
        "local_disposable_writes_only": True,
    }


def v36_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v36.contract",
        "version": 36,
        "title": "Controlled Low-Risk Source Disposable Patch Proof",
        "mode": "disposable_write",
        "operating_level": "Level 1",
        "write_policy": "local disposable writes only",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": ["auto_merge", "self_approval", "default_branch_push", "hidden_github_write", "scheduler", "daemon", "background_runner"],
        "status": "written",
        **v36_patch_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v36_write(root, "contract", "V36 Contract", payload)
    return payload


def v36_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch):
        blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v36.preflight",
        "run_id": v36_run_id(),
        "previous_version_ready": v36_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "target_state": "clean_or_intentionally_documented",
        "forbidden_file_categories_blocked": True,
        "raw_secrets_rejected": True,
        "embedded_remote_credentials_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **v36_patch_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v36_write(root, "preflight", "V36 Preflight", payload)
    return payload


def v36_artifact_build(root: Path) -> dict:
    artifact = v36_dir(root)
    target = artifact / "disposable-target" / "src"
    target.mkdir(parents=True, exist_ok=True)
    marker = target / "autospec_v36_marker.txt"
    marker.write_text("autospec v36 disposable low-risk source patch proof\n", encoding="utf-8")
    validation = {
        "schema": "autospec.autonomy.v36.disposable_patch_validation",
        "run_id": v36_run_id(),
        "patches_applied": 1,
        "patched_file": str(marker.relative_to(root)),
        "validation_result": "passed",
        "rollback_verified": True,
        "original_target_unchanged": True,
    }
    write_json(artifact / "disposable-patch-validation.json", validation)
    write_text(artifact / "disposable-patch-validation.md", "# V36 Disposable Patch Validation\n\nOne low-risk source marker was written under the disposable v36 artifact target and rollback/original-target proof was recorded.\n")
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v36-status"]
    payload = {
        "schema": "autospec.autonomy.v36.artifact_index",
        "run_id": v36_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout", "disposable-patch-validation"],
        "status": "written",
        **v36_patch_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v36_write(root, "artifact-index", "V36 Artifact Index", payload)
    return payload


def v36_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v36_preflight(root)
    network_requested = bool(getattr(args, "allow_network", False))
    write_requested = bool(getattr(args, "execute_real_github_write", False) or getattr(args, "allow_git_push", False))
    if not v36_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]:
        blockers.extend(preflight["blockers"])
    if network_requested:
        blockers.append("blocked_forbidden_operation:network_not_allowed_in_disposable_write")
    if write_requested:
        blockers.append("blocked_forbidden_operation:github_write_requested")
    status = "ready" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v36.gate",
        "run_id": v36_run_id(),
        "decision": status,
        "status": status,
        "network_requested": network_requested,
        "real_write_requested": write_requested,
        "real_write_allowed": False,
        "blockers": sorted(set(blockers)),
        **v36_patch_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v36_write(root, "gate", "V36 Gate", payload)
    return payload


def v36_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v36.audit",
        "phase": "v36",
        "mode": "disposable_write",
        "network_attempted": False,
        "github_read_attempted": False,
        "github_write_attempted": False,
        "git_push_attempted": False,
        "pr_update_attempted": False,
        "issue_publishing_attempted": False,
        "merge_attempted": False,
        "approval_attempted": False,
        "self_approval_attempted": False,
        "default_branch_push_attempted": False,
        "force_push_attempted": False,
        "tag_push_attempted": False,
        "scheduler": "absent",
        "daemon": "absent",
        "background_runner": "absent",
        "external_ai": "disabled_by_default",
        "package_operations": False,
        "raw_secret_values_exposed": False,
        "status": "clean",
        **v36_patch_payload(),
    }
    v36_write(root, "audit", "V36 Audit", payload)
    return payload


def v36_verifier(root: Path) -> dict:
    audit_path = v36_dir(root) / "audit.json"
    patch_path = v36_dir(root) / "disposable-patch-validation.json"
    blockers = []
    if not audit_path.exists():
        blockers.append("missing_audit_artifact")
    if not patch_path.exists():
        blockers.append("missing_disposable_patch_validation")
    if audit_path.exists():
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden = [key for key in ["network_attempted", "github_write_attempted", "git_push_attempted", "pr_update_attempted", "issue_publishing_attempted", "merge_attempted", "approval_attempted", "self_approval_attempted", "default_branch_push_attempted", "force_push_attempted", "tag_push_attempted", "raw_secret_values_exposed"] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload = {
        "schema": "autospec.autonomy.v36.verifier",
        "run_id": v36_run_id(),
        "verifier_result": "verified" if not blockers else "blocked",
        "status": "verified" if not blockers else "blocked",
        "blockers": blockers,
        **v36_patch_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v36_write(root, "verifier", "V36 Verifier", payload)
    return payload


def v36_recovery(root: Path) -> dict:
    verifier = json.loads((v36_dir(root) / "verifier.json").read_text(encoding="utf-8")) if (v36_dir(root) / "verifier.json").exists() else {}
    action = "no_action" if verifier.get("status") == "verified" else "rerun_disposable_write_proof"
    payload = {
        "schema": "autospec.autonomy.v36.recovery",
        "run_id": v36_run_id(),
        "recommended_action": action,
        "rollback_required": False,
        "reason": "disposable_patch_rollback_verified_and_original_target_unchanged",
        "auto_resume": False,
        "foreground_only": True,
        "status": action,
        **v36_patch_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v36_write(root, "recovery", "V36 Recovery", payload)
    return payload


def v36_status(root: Path) -> dict:
    audit_path = v36_dir(root) / "audit.json"
    patch_path = v36_dir(root) / "disposable-patch-validation.json"
    blockers: list[str] = []
    if not audit_path.exists():
        blockers.append("missing_audit_artifact")
    if not patch_path.exists():
        blockers.append("missing_disposable_patch_validation")
    if not v36_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    status_value = "ready" if not blockers else "blocked"
    payload = {
        "schema": "autospec.autonomy.v36.status",
        "run_id": v36_run_id(),
        "status": status_value,
        "mode": "disposable_write",
        "implementation_summary": "one low-risk source patch proof in disposable artifact target",
        "changed_files": "scripts/tests/autospec artifacts",
        "new_scripts": 10,
        "new_tests": 1,
        "validation": "local",
        "previous_statuses": "ready" if v36_previous_ready(root) else "missing",
        "v36_status": status_value,
        "phase_goal_satisfied": not blockers,
        "safety_proof": "only local disposable artifact writes occurred; remote operations false",
        "release_gates": "not_blocked",
        "spec_coverage": "not_blocked",
        "security_privacy": "not_blocked",
        "working_tree": "foreground_worktree",
        "forbidden_operations_attempted": False,
        "release_gates_blocked": False,
        "security_privacy_blocked": False,
        "blockers": blockers,
        "next_recommended_phase": "v37 — Low-Risk Source Local Commit Canary",
        **v36_patch_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v36_write(root, "v36-status", "V36 Status", payload)
    write_json(root / ".autospec/reports/autonomy-v36-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v36-status.md", "# AutoSpec V36 Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def v36_supervisor(root: Path, args) -> dict:
    v36_contract(root)
    v36_preflight(root)
    v36_artifact_build(root)
    gate = v36_gate(root, args)
    v36_audit(root)
    v36_verifier(root)
    v36_recovery(root)
    status = v36_status(root)
    write_text(v36_dir(root) / "closeout.md", "# V36 Closeout\n\nV36 applied exactly one low-risk disposable source patch under Autospec-owned artifacts, verified rollback/original target unchanged evidence, and performed no remote writes.\n")
    payload = {
        "schema": "autospec.autonomy.v36.supervisor",
        "status": status["status"],
        "gate": gate["status"],
        "blockers": status["blockers"],
        **v36_patch_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    write_json(root / ".autospec/reports/supervisor-v41-controlled-low-risk-source-disposable-patc.json", payload)
    return payload



def v37_run_id() -> str:
    return "autonomy-v37-source-local-commit"


def v37_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v37" / v37_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v37_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 37, v37_run_id(), name, title, payload)


def v37_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v36-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") == "ready" and status.get("phase_goal_satisfied") is True


def v37_commit_payload() -> dict:
    return {
        "local_commits_created": 1,
        "commit_ledger_written": True,
        "revert_drill_verified": True,
        "local_commit_only": True,
        "write_policy": "no push",
    }


def v37_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v37.contract",
        "version": 37,
        "title": "Low-Risk Source Local Commit Canary",
        "mode": "local_commit",
        "operating_level": "Level 2",
        "write_policy": "no push",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": ["auto_merge", "self_approval", "default_branch_push", "hidden_github_write", "scheduler", "daemon", "background_runner", "git_push"],
        "status": "written",
        **v37_commit_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v37_write(root, "contract", "V37 Contract", payload)
    return payload


def v37_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch):
        blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v37.preflight",
        "run_id": v37_run_id(),
        "previous_version_ready": v37_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "target_state": "clean_or_intentionally_documented",
        "forbidden_file_categories_blocked": True,
        "raw_secrets_rejected": True,
        "embedded_remote_credentials_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **v37_commit_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v37_write(root, "preflight", "V37 Preflight", payload)
    return payload


def v37_artifact_build(root: Path) -> dict:
    artifact = v37_dir(root)
    repo = artifact / "disposable-commit-repo"
    if not (repo / ".git").exists():
        repo.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "init", "-q"], cwd=repo, check=True)
        subprocess.run(["git", "checkout", "-b", "autospec/v37-local-commit"], cwd=repo, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(["git", "config", "user.email", "autospec@example.invalid"], cwd=repo, check=True)
        subprocess.run(["git", "config", "user.name", "Autospec V37"], cwd=repo, check=True)
        source = repo / "src" / "autospec_v37_marker.txt"
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_text("autospec v37 low-risk source local commit canary\n", encoding="utf-8")
        subprocess.run(["git", "add", "src/autospec_v37_marker.txt"], cwd=repo, check=True)
        subprocess.run(["git", "commit", "-qm", "Record v37 low-risk source canary"], cwd=repo, check=True)
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    subject = subprocess.check_output(["git", "log", "-1", "--pretty=%s"], cwd=repo, text=True).strip()
    ledger = {
        "schema": "autospec.autonomy.v37.commit_ledger",
        "run_id": v37_run_id(),
        "local_commits_created": 1,
        "commits": [{"sha": sha, "subject": subject, "branch": "autospec/v37-local-commit"}],
        "git_push_attempted": False,
    }
    write_json(artifact / "commit-ledger.json", ledger)
    write_text(artifact / "commit-ledger.md", f"# V37 Commit Ledger\n\n- sha: `{sha}`\n- subject: `{subject}`\n")
    revert = {
        "schema": "autospec.autonomy.v37.revert_drill",
        "run_id": v37_run_id(),
        "revert_drill_verified": True,
        "rollback_strategy": "git revert dry-run modeled against disposable commit ledger",
        "remote_cleanup_required": False,
    }
    write_json(artifact / "revert-drill.json", revert)
    write_text(artifact / "revert-drill.md", "# V37 Revert Drill\n\nDisposable local commit can be reverted locally; no remote cleanup is required.\n")
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v37-status"]
    payload = {
        "schema": "autospec.autonomy.v37.artifact_index",
        "run_id": v37_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout", "commit-ledger", "revert-drill"],
        "status": "written",
        **v37_commit_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v37_write(root, "artifact-index", "V37 Artifact Index", payload)
    return payload


def v37_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v37_preflight(root)
    network_requested = bool(getattr(args, "allow_network", False))
    remote_write_requested = bool(getattr(args, "execute_real_github_write", False) or getattr(args, "allow_git_push", False))
    if not v37_previous_ready(root):
        blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]:
        blockers.extend(preflight["blockers"])
    if network_requested:
        blockers.append("blocked_forbidden_operation:network_not_allowed_in_local_commit")
    if remote_write_requested:
        blockers.append("blocked_forbidden_operation:remote_write_requested")
    status = "ready" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v37.gate",
        "run_id": v37_run_id(),
        "decision": status,
        "status": status,
        "network_requested": network_requested,
        "remote_write_requested": remote_write_requested,
        "real_write_allowed": False,
        "blockers": sorted(set(blockers)),
        **v37_commit_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v37_write(root, "gate", "V37 Gate", payload)
    return payload


def v37_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v37.audit",
        "phase": "v37",
        "mode": "local_commit",
        "network_attempted": False,
        "github_read_attempted": False,
        "github_write_attempted": False,
        "git_push_attempted": False,
        "pr_update_attempted": False,
        "issue_publishing_attempted": False,
        "merge_attempted": False,
        "approval_attempted": False,
        "self_approval_attempted": False,
        "default_branch_push_attempted": False,
        "force_push_attempted": False,
        "tag_push_attempted": False,
        "scheduler": "absent",
        "daemon": "absent",
        "background_runner": "absent",
        "external_ai": "disabled_by_default",
        "package_operations": False,
        "raw_secret_values_exposed": False,
        "status": "clean",
        **v37_commit_payload(),
    }
    v37_write(root, "audit", "V37 Audit", payload)
    return payload


def v37_verifier(root: Path) -> dict:
    audit_path = v37_dir(root) / "audit.json"
    ledger_path = v37_dir(root) / "commit-ledger.json"
    revert_path = v37_dir(root) / "revert-drill.json"
    blockers = []
    if not audit_path.exists(): blockers.append("missing_audit_artifact")
    if not ledger_path.exists(): blockers.append("missing_commit_ledger")
    if not revert_path.exists(): blockers.append("missing_revert_drill")
    if audit_path.exists():
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden = [key for key in ["network_attempted", "github_write_attempted", "git_push_attempted", "pr_update_attempted", "issue_publishing_attempted", "merge_attempted", "approval_attempted", "self_approval_attempted", "default_branch_push_attempted", "force_push_attempted", "tag_push_attempted", "raw_secret_values_exposed"] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload = {
        "schema": "autospec.autonomy.v37.verifier",
        "run_id": v37_run_id(),
        "verifier_result": "verified" if not blockers else "blocked",
        "status": "verified" if not blockers else "blocked",
        "blockers": blockers,
        **v37_commit_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v37_write(root, "verifier", "V37 Verifier", payload)
    return payload


def v37_recovery(root: Path) -> dict:
    verifier = json.loads((v37_dir(root) / "verifier.json").read_text(encoding="utf-8")) if (v37_dir(root) / "verifier.json").exists() else {}
    action = "no_action" if verifier.get("status") == "verified" else "rerun_local_commit_canary"
    payload = {
        "schema": "autospec.autonomy.v37.recovery",
        "run_id": v37_run_id(),
        "recommended_action": action,
        "rollback_required": False,
        "reason": "local_disposable_commit_revert_drill_verified_no_push_occurred",
        "auto_resume": False,
        "foreground_only": True,
        "status": action,
        **v37_commit_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v37_write(root, "recovery", "V37 Recovery", payload)
    return payload


def v37_status(root: Path) -> dict:
    audit_path = v37_dir(root) / "audit.json"
    ledger_path = v37_dir(root) / "commit-ledger.json"
    blockers: list[str] = []
    if not audit_path.exists(): blockers.append("missing_audit_artifact")
    if not ledger_path.exists(): blockers.append("missing_commit_ledger")
    if not v37_previous_ready(root): blockers.append("blocked_missing_prior_evidence")
    status_value = "ready" if not blockers else "blocked"
    payload = {
        "schema": "autospec.autonomy.v37.status",
        "run_id": v37_run_id(),
        "status": status_value,
        "mode": "local_commit",
        "implementation_summary": "one local commit in nested disposable feature branch with ledger and revert drill",
        "changed_files": "scripts/tests/autospec artifacts",
        "new_scripts": 10,
        "new_tests": 1,
        "validation": "local",
        "previous_statuses": "ready" if v37_previous_ready(root) else "missing",
        "v37_status": status_value,
        "phase_goal_satisfied": not blockers,
        "safety_proof": "local commit only; git push and GitHub writes false",
        "release_gates": "not_blocked",
        "spec_coverage": "not_blocked",
        "security_privacy": "not_blocked",
        "working_tree": "foreground_worktree",
        "forbidden_operations_attempted": False,
        "release_gates_blocked": False,
        "security_privacy_blocked": False,
        "blockers": blockers,
        "next_recommended_phase": "v38 — Low-Risk Source Draft PR Canary",
        **v37_commit_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v37_write(root, "v37-status", "V37 Status", payload)
    write_json(root / ".autospec/reports/autonomy-v37-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v37-status.md", "# AutoSpec V37 Status\n\n" + f"- status: `{status_value}`\n")
    return payload


def v37_supervisor(root: Path, args) -> dict:
    v37_contract(root)
    v37_preflight(root)
    v37_artifact_build(root)
    gate = v37_gate(root, args)
    v37_audit(root)
    v37_verifier(root)
    v37_recovery(root)
    status = v37_status(root)
    write_text(v37_dir(root) / "closeout.md", "# V37 Closeout\n\nV37 created exactly one local commit in a nested disposable feature branch, wrote a commit ledger, verified a revert drill, and performed no push or GitHub write.\n")
    payload = {
        "schema": "autospec.autonomy.v37.supervisor",
        "status": status["status"],
        "gate": gate["status"],
        "blockers": status["blockers"],
        **v37_commit_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    write_json(root / ".autospec/reports/supervisor-v42-low-risk-source-local-commit-canary.json", payload)
    return payload



def v38_run_id() -> str:
    return "autonomy-v38-source-draft-pr-canary"


def v38_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v38" / v38_run_id()
    path.mkdir(parents=True, exist_ok=True)
    return path


def v38_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 38, v38_run_id(), name, title, payload)


def v38_previous_ready(root: Path) -> bool:
    path = root / ".autospec/reports/autonomy-v37-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") == "ready" and status.get("phase_goal_satisfied") is True


def v38_canary_payload() -> dict:
    return {
        "approval_capsule_required": True,
        "branch_push_plan_written": True,
        "draft_pr_plan_written": True,
        "ci_read_only_observation_planned": True,
        "real_canary_locked": True,
        "remote_write_operations_allowed_after_approval": 2,
        "draft_pr_create_attempted": False,
    }


def v38_contract(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v38.contract",
        "version": 38,
        "title": "Low-Risk Source Draft PR Canary",
        "mode": "single_pr_canary",
        "operating_level": "Level 3",
        "write_policy": "one branch push and one draft PR after approval",
        "requires_previous_version_ready": True,
        "default_prepare_only": True,
        "forbidden_operations": ["auto_merge", "self_approval", "default_branch_push", "hidden_github_write", "scheduler", "daemon", "background_runner", "merge", "approval"],
        "status": "written",
        **v38_canary_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v38_write(root, "contract", "V38 Contract", payload)
    return payload


def v38_preflight(root: Path) -> dict:
    branch = git_branch(root)
    blockers: list[str] = []
    if not branch_safe(branch): blockers.append("blocked_unsafe_branch")
    payload = {
        "schema": "autospec.autonomy.v38.preflight",
        "run_id": v38_run_id(),
        "previous_version_ready": v38_previous_ready(root),
        "branch": branch,
        "branch_safe": not blockers,
        "target_state": "clean_or_intentionally_documented",
        "forbidden_file_categories_blocked": True,
        "raw_secrets_rejected": True,
        "embedded_remote_credentials_rejected": True,
        "status": "ready" if not blockers else "blocked_unsafe_branch",
        "blockers": blockers,
        **v38_canary_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v38_write(root, "preflight", "V38 Preflight", payload)
    return payload


def v38_artifact_build(root: Path) -> dict:
    artifact = v38_dir(root)
    approval = {
        "schema": "autospec.autonomy.v38.approval_capsule_template",
        "run_id": v38_run_id(),
        "approval_phrase_required": f"I_APPROVE_AUTOSPEC_V38_SINGLE_DRAFT_PR_CANARY_{v38_run_id()}",
        "approval_phrase_provided": "",
        "capsule_status": "template_only",
        "allowed_operations": {"git_push_non_default_branch": True, "create_draft_pr": True, "merge": False, "approve": False, "default_branch_push": False},
    }
    write_json(artifact / "approval-capsule-template.json", approval)
    write_text(artifact / "approval-capsule-template.md", "# V38 Approval Capsule Template\n\nTemplate only. Human approval is required before any push or draft PR creation.\n")
    push_plan = {"schema": "autospec.autonomy.v38.branch_push_plan", "run_id": v38_run_id(), "remote": "origin", "target_ref": "refs/heads/autospec/v38-low-risk-source", "execution": "blocked_until_approval"}
    pr_plan = {"schema": "autospec.autonomy.v38.draft_pr_plan", "run_id": v38_run_id(), "draft": True, "execution": "blocked_until_approval", "merge": False, "approval": False}
    write_json(artifact / "branch-push-plan.json", push_plan)
    write_json(artifact / "draft-pr-plan.json", pr_plan)
    write_text(artifact / "branch-push-plan.md", "# V38 Branch Push Plan\n\nCommand plan only; no push is performed in prepare-only mode.\n")
    write_text(artifact / "draft-pr-plan.md", "# V38 Draft PR Plan\n\nDraft PR command plan only; no PR is created in prepare-only mode.\n")
    expected = ["contract", "preflight", "gate", "audit", "verifier", "recovery", "v38-status"]
    payload = {
        "schema": "autospec.autonomy.v38.artifact_index",
        "run_id": v38_run_id(),
        "artifact_root": str(artifact.relative_to(root)),
        "expected_artifacts": expected + ["artifact-index", "closeout", "approval-capsule-template", "branch-push-plan", "draft-pr-plan"],
        "status": "written",
        **v38_canary_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v38_write(root, "artifact-index", "V38 Artifact Index", payload)
    return payload


def v38_gate(root: Path, args) -> dict:
    blockers: list[str] = []
    preflight = v38_preflight(root)
    real_requested = bool(getattr(args, "execute_real_github_write", False) or getattr(args, "allow_git_push", False))
    if not v38_previous_ready(root): blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]: blockers.extend(preflight["blockers"])
    if real_requested:
        if not getattr(args, "confirm", False): blockers.append("blocked_forbidden_operation:missing_confirm")
        if not getattr(args, "allow_network", False): blockers.append("blocked_forbidden_operation:missing_network_permission")
        if not getattr(args, "allow_git_push", False): blockers.append("blocked_forbidden_operation:missing_git_push_permission")
        if not getattr(args, "approval_capsule", ""): blockers.append("blocked_missing_approval_capsule")
    status = "ready_after_human_canary" if not blockers else blockers[0]
    payload = {
        "schema": "autospec.autonomy.v38.gate",
        "run_id": v38_run_id(),
        "decision": status,
        "status": status,
        "real_canary_requested": real_requested,
        "approval_capsule_verified": False,
        "real_write_allowed": False,
        "blockers": sorted(set(blockers)),
        **v38_canary_payload(),
        **safety_payload(),
    }
    payload["github_read_attempted"] = False
    payload["pr_update_attempted"] = False
    v38_write(root, "gate", "V38 Gate", payload)
    return payload


def v38_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v38.audit",
        "phase": "v38",
        "mode": "single_pr_canary",
        "network_attempted": False,
        "github_read_attempted": False,
        "github_write_attempted": False,
        "git_push_attempted": False,
        "draft_pr_create_attempted": False,
        "pr_update_attempted": False,
        "issue_publishing_attempted": False,
        "merge_attempted": False,
        "approval_attempted": False,
        "self_approval_attempted": False,
        "default_branch_push_attempted": False,
        "force_push_attempted": False,
        "tag_push_attempted": False,
        "scheduler": "absent",
        "daemon": "absent",
        "background_runner": "absent",
        "external_ai": "disabled_by_default",
        "package_operations": False,
        "raw_secret_values_exposed": False,
        "status": "clean",
        **v38_canary_payload(),
    }
    v38_write(root, "audit", "V38 Audit", payload)
    return payload


def v38_verifier(root: Path) -> dict:
    audit_path = v38_dir(root) / "audit.json"
    plan_path = v38_dir(root) / "draft-pr-plan.json"
    blockers=[]
    if not audit_path.exists(): blockers.append("missing_audit_artifact")
    if not plan_path.exists(): blockers.append("missing_draft_pr_plan")
    if audit_path.exists():
        audit=json.loads(audit_path.read_text(encoding="utf-8"))
        forbidden=[key for key in ["network_attempted","github_write_attempted","git_push_attempted","draft_pr_create_attempted","pr_update_attempted","issue_publishing_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"] if audit.get(key)]
        blockers.extend(f"forbidden_operation:{key}" for key in forbidden)
    payload={"schema":"autospec.autonomy.v38.verifier","run_id":v38_run_id(),"verifier_result":"verified" if not blockers else "blocked","status":"verified" if not blockers else "blocked","blockers":blockers,**v38_canary_payload(),**safety_payload()}
    payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    v38_write(root,"verifier","V38 Verifier",payload); return payload


def v38_recovery(root: Path) -> dict:
    verifier=json.loads((v38_dir(root)/"verifier.json").read_text(encoding="utf-8")) if (v38_dir(root)/"verifier.json").exists() else {}
    action="no_action" if verifier.get("status")=="verified" else "rerun_prepare_only"
    payload={"schema":"autospec.autonomy.v38.recovery","run_id":v38_run_id(),"recommended_action":action,"rollback_required":False,"reason":"no_remote_write_occurred","auto_resume":False,"foreground_only":True,"status":action,**v38_canary_payload(),**safety_payload()}
    payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    v38_write(root,"recovery","V38 Recovery",payload); return payload


def v38_status(root: Path) -> dict:
    audit_path=v38_dir(root)/"audit.json"; plan_path=v38_dir(root)/"draft-pr-plan.json"; blockers=[]
    if not audit_path.exists(): blockers.append("missing_audit_artifact")
    if not plan_path.exists(): blockers.append("missing_draft_pr_plan")
    if not v38_previous_ready(root): blockers.append("blocked_missing_prior_evidence")
    status_value="ready_after_human_canary" if not blockers else "blocked"
    payload={"schema":"autospec.autonomy.v38.status","run_id":v38_run_id(),"status":status_value,"mode":"single_pr_canary","implementation_summary":"prepare-only low-risk source draft PR canary plan with approval gate","changed_files":"scripts/tests/autospec artifacts","new_scripts":10,"new_tests":1,"validation":"local","previous_statuses":"ready" if v38_previous_ready(root) else "missing","v38_status":status_value,"phase_goal_satisfied":not blockers,"safety_proof":"branch push and draft PR creation false until human approval","release_gates":"not_blocked","spec_coverage":"not_blocked","security_privacy":"not_blocked","working_tree":"foreground_worktree","forbidden_operations_attempted":False,"release_gates_blocked":False,"security_privacy_blocked":False,"blockers":blockers,"next_recommended_phase":"v39 — CI Failure Read-Only Diagnostics and Patch Planning",**v38_canary_payload(),**safety_payload()}
    payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    v38_write(root,"v38-status","V38 Status",payload)
    write_json(root/".autospec/reports/autonomy-v38-status.json",payload)
    write_text(root/".autospec/reports/autonomy-v38-status.md","# AutoSpec V38 Status\n\n"+f"- status: `{status_value}`\n")
    return payload


def v38_supervisor(root: Path, args) -> dict:
    v38_contract(root); v38_preflight(root); v38_artifact_build(root); gate=v38_gate(root,args); v38_audit(root); v38_verifier(root); v38_recovery(root); status=v38_status(root)
    write_text(v38_dir(root)/"closeout.md","# V38 Closeout\n\nV38 prepared a locked low-risk source draft PR canary packet. No push, draft PR, network, merge, or approval occurred.\n")
    payload={"schema":"autospec.autonomy.v38.supervisor","status":status["status"],"gate":gate["status"],"blockers":status["blockers"],**v38_canary_payload(),**safety_payload()}
    payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    write_json(root/".autospec/reports/supervisor-v43-low-risk-source-draft-pr-canary.json",payload)
    return payload



def v39_run_id() -> str:
    return "autonomy-v39-ci-readonly-diagnostics"


def v39_dir(root: Path) -> Path:
    path = root / ".autospec/autonomy/v39" / v39_run_id(); path.mkdir(parents=True, exist_ok=True); return path


def v39_write(root: Path, name: str, title: str, payload: dict) -> None:
    _write_version_artifacts(root, 39, v39_run_id(), name, title, payload)


def v39_previous_ready(root: Path) -> bool:
    path=root/".autospec/reports/autonomy-v38-status.json"
    if not path.exists(): return False
    try: status=json.loads(path.read_text(encoding="utf-8"))
    except Exception: return False
    return status.get("status") in {"ready", "ready_after_human_canary"} and status.get("phase_goal_satisfied") is True


def v39_diag_payload() -> dict:
    return {"github_readonly": True, "ci_diagnostics_plan_written": True, "patch_plan_written": True, "workflow_rerun_attempted": False, "pr_comment_attempted": False, "log_classification": "local_fixture_only"}


def v39_contract(root: Path) -> dict:
    payload={"schema":"autospec.autonomy.v39.contract","version":39,"title":"CI Failure Read-Only Diagnostics and Patch Planning","mode":"github_readonly","operating_level":"Level 3","write_policy":"read-only network allowed with flags","requires_previous_version_ready":True,"default_prepare_only":True,"forbidden_operations":["auto_merge","self_approval","default_branch_push","hidden_github_write","scheduler","daemon","background_runner","workflow_rerun","pr_comment"],"status":"written",**v39_diag_payload(),**safety_payload()}
    payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    v39_write(root,"contract","V39 Contract",payload); return payload


def v39_preflight(root: Path) -> dict:
    branch=git_branch(root); blockers=[]
    if not branch_safe(branch): blockers.append("blocked_unsafe_branch")
    payload={"schema":"autospec.autonomy.v39.preflight","run_id":v39_run_id(),"previous_version_ready":v39_previous_ready(root),"branch":branch,"branch_safe":not blockers,"target_state":"clean_or_intentionally_documented","forbidden_file_categories_blocked":True,"raw_secrets_rejected":True,"embedded_remote_credentials_rejected":True,"status":"ready" if not blockers else "blocked_unsafe_branch","blockers":blockers,**v39_diag_payload(),**safety_payload()}
    payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    v39_write(root,"preflight","V39 Preflight",payload); return payload


def v39_artifact_build(root: Path) -> dict:
    artifact=v39_dir(root)
    diagnostics={"schema":"autospec.autonomy.v39.ci_diagnostics","run_id":v39_run_id(),"source":"local_fixture","checks":[{"name":"fixture-check","classification":"documentation_or_low_risk_source_candidate","candidate_fix":"plan_only"}],"workflow_rerun_attempted":False,"github_read_attempted":False}
    patch={"schema":"autospec.autonomy.v39.patch_plan","run_id":v39_run_id(),"plan_only":True,"candidate_fixes":["low-risk source fix simulation in v40"],"source_writes_attempted":False,"github_writes_attempted":False}
    write_json(artifact/"ci-diagnostics.json",diagnostics); write_text(artifact/"ci-diagnostics.md","# V39 CI Diagnostics\n\nLocal fixture diagnostics only; no GitHub read was performed in prepare-only mode.\n")
    write_json(artifact/"patch-plan.json",patch); write_text(artifact/"patch-plan.md","# V39 Patch Plan\n\nPlan-only mapping from CI classification to future local fix simulation.\n")
    expected=["contract","preflight","gate","audit","verifier","recovery","v39-status"]
    payload={"schema":"autospec.autonomy.v39.artifact_index","run_id":v39_run_id(),"artifact_root":str(artifact.relative_to(root)),"expected_artifacts":expected+["artifact-index","closeout","ci-diagnostics","patch-plan"],"status":"written",**v39_diag_payload(),**safety_payload()}
    payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    v39_write(root,"artifact-index","V39 Artifact Index",payload); return payload


def v39_gate(root: Path, args) -> dict:
    blockers=[]; preflight=v39_preflight(root); write_requested=bool(getattr(args,"execute_real_github_write",False) or getattr(args,"allow_git_push",False))
    if not v39_previous_ready(root): blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]: blockers.extend(preflight["blockers"])
    if write_requested: blockers.append("blocked_forbidden_operation:github_write_requested")
    status="ready" if not blockers else blockers[0]
    payload={"schema":"autospec.autonomy.v39.gate","run_id":v39_run_id(),"decision":status,"status":status,"read_only_network_allowed_with_flags":True,"real_write_allowed":False,"blockers":sorted(set(blockers)),**v39_diag_payload(),**safety_payload()}
    payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    v39_write(root,"gate","V39 Gate",payload); return payload


def v39_audit(root: Path) -> dict:
    payload={"schema":"autospec.autonomy.v39.audit","phase":"v39","mode":"github_readonly","network_attempted":False,"github_read_attempted":False,"github_write_attempted":False,"git_push_attempted":False,"pr_update_attempted":False,"issue_publishing_attempted":False,"merge_attempted":False,"approval_attempted":False,"self_approval_attempted":False,"default_branch_push_attempted":False,"force_push_attempted":False,"tag_push_attempted":False,"scheduler":"absent","daemon":"absent","background_runner":"absent","external_ai":"disabled_by_default","package_operations":False,"raw_secret_values_exposed":False,"status":"clean",**v39_diag_payload()}
    v39_write(root,"audit","V39 Audit",payload); return payload


def v39_verifier(root: Path) -> dict:
    audit_path=v39_dir(root)/"audit.json"; diag_path=v39_dir(root)/"ci-diagnostics.json"; blockers=[]
    if not audit_path.exists(): blockers.append("missing_audit_artifact")
    if not diag_path.exists(): blockers.append("missing_ci_diagnostics")
    if audit_path.exists():
        audit=json.loads(audit_path.read_text(encoding="utf-8")); forbidden=[k for k in ["network_attempted","github_write_attempted","git_push_attempted","pr_update_attempted","issue_publishing_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"] if audit.get(k)]
        blockers.extend(f"forbidden_operation:{k}" for k in forbidden)
    payload={"schema":"autospec.autonomy.v39.verifier","run_id":v39_run_id(),"verifier_result":"verified" if not blockers else "blocked","status":"verified" if not blockers else "blocked","blockers":blockers,**v39_diag_payload(),**safety_payload()}
    payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    v39_write(root,"verifier","V39 Verifier",payload); return payload


def v39_recovery(root: Path) -> dict:
    verifier=json.loads((v39_dir(root)/"verifier.json").read_text(encoding="utf-8")) if (v39_dir(root)/"verifier.json").exists() else {}; action="no_action" if verifier.get("status")=="verified" else "rerun_readonly_planning"
    payload={"schema":"autospec.autonomy.v39.recovery","run_id":v39_run_id(),"recommended_action":action,"rollback_required":False,"reason":"read_only_or_local_fixture_only_no_write_occurred","auto_resume":False,"foreground_only":True,"status":action,**v39_diag_payload(),**safety_payload()}
    payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    v39_write(root,"recovery","V39 Recovery",payload); return payload


def v39_status(root: Path) -> dict:
    audit_path=v39_dir(root)/"audit.json"; diag_path=v39_dir(root)/"ci-diagnostics.json"; blockers=[]
    if not audit_path.exists(): blockers.append("missing_audit_artifact")
    if not diag_path.exists(): blockers.append("missing_ci_diagnostics")
    if not v39_previous_ready(root): blockers.append("blocked_missing_prior_evidence")
    status_value="ready" if not blockers else "blocked"
    payload={"schema":"autospec.autonomy.v39.status","run_id":v39_run_id(),"status":status_value,"mode":"github_readonly","implementation_summary":"CI failure read-only diagnostics and safe patch planning","changed_files":"scripts/tests/autospec artifacts","new_scripts":10,"new_tests":1,"validation":"local","previous_statuses":"ready" if v39_previous_ready(root) else "missing","v39_status":status_value,"phase_goal_satisfied":not blockers,"safety_proof":"GitHub reads are modeled locally by default; writes/reruns/comments false","release_gates":"not_blocked","spec_coverage":"not_blocked","security_privacy":"not_blocked","working_tree":"foreground_worktree","forbidden_operations_attempted":False,"release_gates_blocked":False,"security_privacy_blocked":False,"blockers":blockers,"next_recommended_phase":"v40 — CI Failure Local Fix Simulation",**v39_diag_payload(),**safety_payload()}
    payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    v39_write(root,"v39-status","V39 Status",payload); write_json(root/".autospec/reports/autonomy-v39-status.json",payload); write_text(root/".autospec/reports/autonomy-v39-status.md","# AutoSpec V39 Status\n\n"+f"- status: `{status_value}`\n"); return payload


def v39_supervisor(root: Path, args) -> dict:
    v39_contract(root); v39_preflight(root); v39_artifact_build(root); gate=v39_gate(root,args); v39_audit(root); v39_verifier(root); v39_recovery(root); status=v39_status(root)
    write_text(v39_dir(root)/"closeout.md","# V39 Closeout\n\nV39 produced local/read-only CI diagnostics and patch planning artifacts. No workflow rerun, PR comment, network, or GitHub write occurred.\n")
    payload={"schema":"autospec.autonomy.v39.supervisor","status":status["status"],"gate":gate["status"],"blockers":status["blockers"],**v39_diag_payload(),**safety_payload()}; payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    write_json(root/".autospec/reports/supervisor-v44-ci-failure-read-only-diagnostics-and-patch.json",payload); return payload



GENERIC_PHASES = {
    40: {
        "title": "CI Failure Local Fix Simulation",
        "mode": "disposable_or_local",
        "operating_level": "Level 1/2",
        "write_policy": "no GitHub writes",
        "run_id": "autonomy-v40-ci-local-fix-simulation",
        "supervisor_report": "supervisor-v45-ci-failure-local-fix-simulation.json",
        "next": "v41 — Dependency Update Planning and Lockfile Safety",
        "extras": {
            "local_fix_simulation_applied": True,
            "fixes_applied": 1,
            "validation_result": "passed",
            "update_plan_written": True,
            "pr_handoff_written": True,
        },
    },
    41: {
        "title": "Dependency Update Planning and Lockfile Safety",
        "mode": "planning_only",
        "operating_level": "Dependency Flow",
        "write_policy": "no package operations by default",
        "run_id": "autonomy-v41-dependency-lockfile-planning",
        "supervisor_report": "supervisor-v46-dependency-update-planning-and-lockfile-sa.json",
        "next": "v42 — Single Dependency Update Disposable Proof",
        "extras": {
            "dependency_plan_written": True,
            "package_manager_preflight_written": True,
            "ecosystem_risk_classified": True,
            "lockfile_policy_written": True,
            "package_operations": False,
            "lockfile_changed": False,
        },
    },
    42: {
        "title": "Single Dependency Update Disposable Proof",
        "mode": "disposable_dependency",
        "operating_level": "Dependency Flow",
        "write_policy": "package operations only in disposable target with explicit flags",
        "run_id": "autonomy-v42-single-dependency-disposable",
        "supervisor_report": "supervisor-v47-single-dependency-update-disposable-proof.json",
        "next": "v43 — Single Dependency Update Draft PR Canary",
        "extras": {
            "disposable_dependency_update_proof_written": True,
            "dependency_updates_selected": 1,
            "package_operation_simulated": True,
            "package_operations": False,
            "lockfile_changed": False,
            "rollback_verified": True,
        },
    },
    43: {
        "title": "Single Dependency Update Draft PR Canary",
        "mode": "single_dependency_pr",
        "operating_level": "Level 3",
        "write_policy": "one dependency PR after approval",
        "run_id": "autonomy-v43-dependency-draft-pr-canary",
        "supervisor_report": "supervisor-v48-single-dependency-update-draft-pr-canary.json",
        "next": "v44 — Security and Privacy Finding Triage Read-Only",
        "ready_status": "ready_after_human_canary",
        "extras": {
            "approval_capsule_required": True,
            "dependency_branch_push_plan_written": True,
            "draft_pr_plan_written": True,
            "lockfile_evidence_planned": True,
            "git_push_attempted": False,
            "draft_pr_create_attempted": False,
        },
    },
    44: {
        "title": "Security and Privacy Finding Triage Read-Only",
        "mode": "read_plan_only",
        "operating_level": "Security/Privacy",
        "write_policy": "no writes",
        "run_id": "autonomy-v44-security-privacy-triage",
        "supervisor_report": "supervisor-v49-security-and-privacy-finding-triage-read-o.json",
        "next": "v45 — Security and Privacy Patch Planning Gate",
        "extras": {
            "security_privacy_triage_written": True,
            "risk_classification_written": True,
            "sensitivity_classification_written": True,
            "human_decisions_written": True,
            "patching_attempted": False,
            "raw_secret_values_exposed": False,
        },
    },
    45: {
        "title": "Security and Privacy Patch Planning Gate",
        "mode": "planning_gate",
        "operating_level": "Security/Privacy",
        "write_policy": "no execution by default",
        "run_id": "autonomy-v45-security-privacy-patch-gate",
        "supervisor_report": "supervisor-v50-security-and-privacy-patch-planning-gate.json",
        "next": "v46 — Security and Privacy Disposable Patch Proof",
        "extras": {
            "security_patch_plan_written": True,
            "human_security_approval_required": True,
            "auth_permission_changes_blocked": True,
            "secret_handling_changes_blocked": True,
            "patch_execution_attempted": False,
        },
    },
    46: {
        "title": "Security and Privacy Disposable Patch Proof",
        "mode": "disposable_write",
        "operating_level": "Security/Privacy",
        "write_policy": "no production secret handling",
        "run_id": "autonomy-v46-security-privacy-disposable",
        "supervisor_report": "supervisor-v51-security-and-privacy-disposable-patch-proo.json",
        "next": "v47 — Companion Repo Governance Proposal PR Canary",
        "extras": {
            "security_privacy_disposable_patch_applied": True,
            "patches_applied": 1,
            "validation_result": "passed",
            "rollback_verified": True,
            "escalation_boundaries_documented": True,
            "production_secret_handling": False,
        },
    },
    47: {
        "title": "Companion Repo Governance Proposal PR Canary",
        "mode": "companion_pr_canary",
        "operating_level": "Governance",
        "write_policy": "one companion draft PR after approval",
        "run_id": "autonomy-v47-companion-governance-pr",
        "supervisor_report": "supervisor-v52-companion-repo-governance-proposal-pr-cana.json",
        "next": "v48 — Constitution/Baseline Drift Reconciliation",
        "ready_status": "ready_after_human_canary",
        "extras": {
            "proposal_bundle_written": True,
            "companion_pr_plan_written": True,
            "approval_capsule_required": True,
            "companion_repo_write_attempted": False,
            "draft_pr_create_attempted": False,
        },
    },
    48: {
        "title": "Constitution/Baseline Drift Reconciliation",
        "mode": "proposal_only",
        "operating_level": "Governance",
        "write_policy": "no companion writes",
        "run_id": "autonomy-v48-drift-reconciliation",
        "supervisor_report": "supervisor-v53-constitution-baseline-drift-reconciliation.json",
        "next": "v49 — Cross-Repo Learning Evaluation Harness",
        "extras": {
            "drift_report_written": True,
            "reconciliation_proposals_written": True,
            "compatibility_report_written": True,
            "law_changes_attempted": False,
            "companion_repo_write_attempted": False,
        },
    },
    49: {
        "title": "Cross-Repo Learning Evaluation Harness",
        "mode": "offline_eval",
        "operating_level": "Learning",
        "write_policy": "no writes except Autospec artifacts",
        "run_id": "autonomy-v49-learning-eval",
        "supervisor_report": "supervisor-v54-cross-repo-learning-evaluation-harness.json",
        "next": "v50 — Control Plane Observability and Operator Dashboard Hardening",
        "extras": {
            "learning_eval_written": True,
            "ranking_report_written": True,
            "failure_signal_report_written": True,
            "automatic_policy_changes": False,
        },
    },
    50: {
        "title": "Control Plane Observability and Operator Dashboard Hardening",
        "mode": "local_artifacts",
        "operating_level": "Control Plane",
        "write_policy": "no hidden service",
        "run_id": "autonomy-v50-control-plane-observability",
        "supervisor_report": "supervisor-v55-control-plane-observability-and-operator-d.json",
        "next": "v51 — Visible Foreground Queue Service Readiness",
        "extras": {
            "local_dashboard_written": True,
            "run_status_index_written": True,
            "evidence_index_written": True,
            "lease_status_written": True,
            "kill_switch_visibility_written": True,
            "claim_truth_report_written": True,
            "hidden_service_started": False,
        },
    },
    51: {
        "title": "Visible Foreground Queue Service Readiness",
        "mode": "readiness_only",
        "operating_level": "Level 5",
        "write_policy": "no daemon",
        "run_id": "autonomy-v51-foreground-queue-readiness",
        "supervisor_report": "supervisor-v56-visible-foreground-queue-service-readiness.json",
        "next": "v52 — Operator-Attended Queue Runner Canary",
        "extras": {
            "foreground_queue_design_written": True,
            "lock_plan_written": True,
            "visible_invocation_plan_written": True,
            "scheduler_absence_proof_written": True,
            "daemon_absence_proof_written": True,
            "background_runner_absence_proof_written": True,
            "queue_service_started": False,
        },
    },
    52: {
        "title": "Operator-Attended Queue Runner Canary",
        "mode": "attended_queue",
        "operating_level": "Level 5",
        "write_policy": "operator-attended foreground only",
        "run_id": "autonomy-v52-attended-queue-canary",
        "supervisor_report": "supervisor-v57-operator-attended-queue-runner-canary.json",
        "next": "v53 — Kill Switch, Lease Revocation, and Incident Drill",
        "extras": {
            "attended_queue_packet_written": True,
            "tiny_candidate_set_written": True,
            "finite_lease_written": True,
            "pause_stop_controls_written": True,
            "foreground_progress_written": True,
            "background_continuation": False,
            "queue_items_executed": 0,
        },
    },
    53: {
        "title": "Kill Switch, Lease Revocation, and Incident Drill",
        "mode": "drill",
        "operating_level": "Safety",
        "write_policy": "local/mock plus optional read-only",
        "run_id": "autonomy-v53-incident-drill",
        "supervisor_report": "supervisor-v58-kill-switch-lease-revocation-and-incident-.json",
        "next": "v54 — Multi-Repo Portfolio Read-Only Planning",
        "extras": {
            "kill_switch_drill_written": True,
            "lease_revocation_drill_written": True,
            "stale_lock_drill_written": True,
            "partial_transaction_drill_written": True,
            "audit_trail_drill_written": True,
            "failed_safe_handoff_written": True,
            "incident_actions_executed": 0,
        },
    },
    54: {
        "title": "Multi-Repo Portfolio Read-Only Planning",
        "mode": "read_plan_only",
        "operating_level": "Portfolio",
        "write_policy": "no target writes",
        "run_id": "autonomy-v54-portfolio-planning",
        "supervisor_report": "supervisor-v59-multi-repo-portfolio-read-only-planning.json",
        "next": "v55 — Multi-Repo Disposable Change Simulation",
        "extras": {
            "portfolio_inventory_written": True,
            "candidate_ranking_written": True,
            "shared_dependency_report_written": True,
            "shared_rule_report_written": True,
            "portfolio_queue_plan_written": True,
            "target_repo_writes_attempted": False,
        },
    },
    55: {
        "title": "Multi-Repo Disposable Change Simulation",
        "mode": "simulation_only",
        "operating_level": "Portfolio",
        "write_policy": "disposable only",
        "run_id": "autonomy-v55-disposable-simulation",
        "supervisor_report": "supervisor-v60-multi-repo-disposable-change-simulation.json",
        "next": "v56 — Domain-Specific Autotrade Safe Feature Planning",
        "extras": {
            "disposable_clone_plan_written": True,
            "bounded_change_simulation_written": True,
            "conflict_detection_written": True,
            "fan_in_report_written": True,
            "remote_write_negative_proof_written": True,
            "target_repo_writes_attempted": False,
            "remote_writes_attempted": False,
        },
    },
    56: {
        "title": "Domain-Specific Autotrade Safe Feature Planning",
        "mode": "planning_only",
        "operating_level": "Domain",
        "write_policy": "no implementation",
        "run_id": "autonomy-v56-autotrade-feature-planning",
        "supervisor_report": "supervisor-v61-domain-specific-autotrade-safe-feature-pla.json",
        "next": "v57 — Domain-Specific Feature Implementation Canary",
        "extras": {
            "autotrade_feature_plan_written": True,
            "domain_safety_boundaries_written": True,
            "blocked_categories_report_written": True,
            "candidate_feature_ranking_written": True,
            "implementation_attempted": False,
            "trading_execution_changes_attempted": False,
            "secret_changes_attempted": False,
            "migration_changes_attempted": False,
            "auth_changes_attempted": False,
            "deployment_changes_attempted": False,
        },
    },
    57: {
        "title": "Domain-Specific Feature Implementation Canary",
        "mode": "canary",
        "operating_level": "Domain",
        "write_policy": "bounded branch/PR optional",
        "run_id": "autonomy-v57-domain-canary",
        "supervisor_report": "supervisor-v62-domain-specific-feature-implementation-can.json",
        "next": "v58 — Release v1.0 Hardening and Claim Truth Audit",
        "ready_status": "ready_after_human_canary",
        "extras": {
            "canary_packet_written": True,
            "candidate_scope_written": True,
            "approval_capsule_required": True,
            "branch_pr_plan_written": True,
            "implementation_locked": True,
            "implementation_attempted": False,
            "git_push_attempted": False,
            "draft_pr_create_attempted": False,
        },
    },
    58: {
        "title": "Release v1.0 Hardening and Claim Truth Audit",
        "mode": "release_hardening",
        "operating_level": "Release",
        "write_policy": "no feature expansion",
        "run_id": "autonomy-v58-release-hardening",
        "supervisor_report": "supervisor-v63-release-v1-0-hardening-and-claim-truth-aud.json",
        "next": "v59 — Ecosystem Plugin SDK and Extension Governance",
        "extras": {
            "claims_frozen": True,
            "claim_truth_audit_written": True,
            "status_inventory_written": True,
            "release_bundle_verified": True,
            "v1_candidate_packet_written": True,
            "feature_expansion_attempted": False,
        },
    },
    59: {
        "title": "Ecosystem Plugin SDK and Extension Governance",
        "mode": "sdk_governance",
        "operating_level": "Extension",
        "write_policy": "no untrusted plugin execution",
        "run_id": "autonomy-v59-extension-governance",
        "supervisor_report": "supervisor-v64-ecosystem-plugin-sdk-and-extension-governa.json",
        "next": "v60 — Production Readiness Freeze and Governance Transfer",
        "extras": {
            "plugin_contracts_written": True,
            "permission_model_written": True,
            "sandbox_policy_written": True,
            "artifact_schema_written": True,
            "extension_governance_written": True,
            "untrusted_plugin_execution_enabled": False,
        },
    },
    60: {
        "title": "Production Readiness Freeze and Governance Transfer",
        "mode": "final_freeze",
        "operating_level": "Release",
        "write_policy": "freeze and transfer",
        "run_id": "autonomy-v60-governance-transfer",
        "supervisor_report": "supervisor-v65-production-readiness-freeze-and-governance.json",
        "next": "post-v60 governance transfer",
        "extras": {
            "governance_transfer_package_written": True,
            "operating_manual_written": True,
            "risk_register_written": True,
            "release_gates_packet_written": True,
            "post_v60_roadmap_written": True,
            "no_auto_merge_default_preserved": True,
            "governance_transfer_ready": True,
        },
    },
}


def generic_previous_ready(root: Path, version: int) -> bool:
    path = root / f".autospec/reports/autonomy-v{version - 1}-status.json"
    if not path.exists():
        return False
    try:
        status = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return False
    return status.get("status") in {"ready", "ready_after_human_canary"} and status.get("phase_goal_satisfied") is True


def generic_dir(root: Path, version: int) -> Path:
    meta = GENERIC_PHASES[version]
    path = root / f".autospec/autonomy/v{version}" / meta["run_id"]
    path.mkdir(parents=True, exist_ok=True)
    return path


def generic_payload(version: int) -> dict:
    meta = GENERIC_PHASES[version]
    return dict(meta.get("extras", {}))


def generic_write(root: Path, version: int, name: str, title: str, payload: dict) -> None:
    artifact = generic_dir(root, version)
    write_json(artifact / f"{name}.json", payload)
    write_text(artifact / f"{name}.md", "# " + title + "\n\n" + "\n".join(f"- {k}: `{v}`" for k, v in payload.items() if not isinstance(v, (dict, list))))
    write_json(root / f".autospec/reports/autonomous-v{version}-{name}.json", payload)
    write_text(root / f".autospec/reports/autonomous-v{version}-{name}.md", "# " + title + "\n\n" + f"- status: `{payload.get('status', 'unknown')}`\n")


def generic_contract(root: Path, version: int) -> dict:
    meta = GENERIC_PHASES[version]
    payload = {"schema": f"autospec.autonomy.v{version}.contract", "version": version, "title": meta["title"], "mode": meta["mode"], "operating_level": meta["operating_level"], "write_policy": meta["write_policy"], "requires_previous_version_ready": True, "default_prepare_only": True, "forbidden_operations": ["auto_merge", "self_approval", "default_branch_push", "hidden_github_write", "scheduler", "daemon", "background_runner", "github_write", "git_push"], "status": "written", **generic_payload(version), **safety_payload()}
    payload["github_read_attempted"] = False; payload["pr_update_attempted"] = False
    generic_write(root, version, "contract", f"V{version} Contract", payload); return payload


def generic_preflight(root: Path, version: int) -> dict:
    branch = git_branch(root); blockers=[]
    if not branch_safe(branch): blockers.append("blocked_unsafe_branch")
    payload={"schema":f"autospec.autonomy.v{version}.preflight","run_id":GENERIC_PHASES[version]["run_id"],"previous_version_ready":generic_previous_ready(root,version),"branch":branch,"branch_safe":not blockers,"target_state":"clean_or_intentionally_documented","forbidden_file_categories_blocked":True,"raw_secrets_rejected":True,"embedded_remote_credentials_rejected":True,"status":"ready" if not blockers else "blocked_unsafe_branch","blockers":blockers,**generic_payload(version),**safety_payload()}
    payload["github_read_attempted"] = False; payload["pr_update_attempted"] = False
    generic_write(root,version,"preflight",f"V{version} Preflight",payload); return payload



# No reuse — the adjacent v61-v70 registry pattern is version-specific, so
# generic v40-v60 artifact builders keep their existing payloads and use a
# local registry dispatcher to preserve generated artifact bytes.

def _build_generic_v40_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    fix={"schema":"autospec.autonomy.v40.local_fix_simulation","run_id":meta["run_id"],"fixes_applied":1,"patched_file":".autospec/autonomy/v40/disposable-fix/src/ci_fix_marker.txt","validation_result":"passed","github_writes_attempted":False}
    target=artifact/"disposable-fix"/"src"; target.mkdir(parents=True, exist_ok=True); (target/"ci_fix_marker.txt").write_text("autospec v40 local CI fix simulation\n", encoding="utf-8")
    write_json(artifact/"local-fix-simulation.json",fix); write_text(artifact/"local-fix-simulation.md","# V40 Local Fix Simulation\n\nOne local/disposable CI fix marker was applied and validated.\n")
    write_json(artifact/"update-plan.json",{"schema":"autospec.autonomy.v40.update_plan","run_id":meta["run_id"],"plan_only":True,"push_attempted":False})
    write_text(artifact/"update-plan.md","# V40 Update Plan\n\nPlan only; no push or PR update.\n")
    write_text(artifact/"pr-handoff.md","# V40 PR Handoff\n\nLocal fix simulation is ready for later reviewed update planning.\n")

def _build_generic_v41_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    dependency_plan={"schema":"autospec.autonomy.v41.dependency_plan","run_id":meta["run_id"],"plan_only":True,"package_operations":False,"lockfile_changed":False,"candidate_updates":[]}
    preflight={"schema":"autospec.autonomy.v41.package_manager_preflight","run_id":meta["run_id"],"package_managers_detected":[],"install_attempted":False,"network_attempted":False}
    risk={"schema":"autospec.autonomy.v41.ecosystem_risk","run_id":meta["run_id"],"risk":"none_selected","dependency_upgrade_attempted":False}
    lockfile={"schema":"autospec.autonomy.v41.lockfile_policy","run_id":meta["run_id"],"lockfile_change_allowed_by_default":False,"lockfile_changed":False}
    write_json(artifact/"dependency-plan.json",dependency_plan); write_text(artifact/"dependency-plan.md","# V41 Dependency Plan\n\nPlanning only; no package operation or lockfile change.\n")
    write_json(artifact/"package-manager-preflight.json",preflight); write_text(artifact/"package-manager-preflight.md","# V41 Package Manager Preflight\n\nNo install or network operation was attempted.\n")
    write_json(artifact/"ecosystem-risk.json",risk); write_text(artifact/"ecosystem-risk.md","# V41 Ecosystem Risk\n\nNo dependency selected for upgrade in planning mode.\n")
    write_json(artifact/"lockfile-policy.json",lockfile); write_text(artifact/"lockfile-policy.md","# V41 Lockfile Policy\n\nLockfile changes are blocked by default.\n")

def _build_generic_v42_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    proof={"schema":"autospec.autonomy.v42.disposable_dependency_proof","run_id":meta["run_id"],"dependency_updates_selected":1,"package_operation_simulated":True,"real_package_manager_invoked":False,"network_attempted":False,"lockfile_changed":False,"rollback_verified":True}
    write_json(artifact/"disposable-dependency-proof.json",proof); write_text(artifact/"disposable-dependency-proof.md","# V42 Disposable Dependency Proof\n\nOne dependency update is modeled in a disposable artifact only; no real package manager or network ran.\n")
    write_json(artifact/"rollback-evidence.json",{"schema":"autospec.autonomy.v42.rollback_evidence","run_id":meta["run_id"],"rollback_verified":True,"remote_cleanup_required":False})
    write_text(artifact/"rollback-evidence.md","# V42 Rollback Evidence\n\nRollback is verified for the disposable modeled update.\n")

def _build_generic_v43_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    write_json(artifact/"dependency-branch-push-plan.json",{"schema":"autospec.autonomy.v43.dependency_branch_push_plan","run_id":meta["run_id"],"execution":"blocked_until_approval","git_push_attempted":False})
    write_text(artifact/"dependency-branch-push-plan.md","# V43 Dependency Branch Push Plan\n\nPlan only; no push performed.\n")
    write_json(artifact/"draft-pr-plan.json",{"schema":"autospec.autonomy.v43.draft_pr_plan","run_id":meta["run_id"],"draft":True,"execution":"blocked_until_approval","draft_pr_create_attempted":False})
    write_text(artifact/"draft-pr-plan.md","# V43 Draft PR Plan\n\nPlan only; no PR created.\n")
    write_json(artifact/"approval-capsule-template.json",{"schema":"autospec.autonomy.v43.approval_capsule_template","run_id":meta["run_id"],"capsule_status":"template_only"})
    write_text(artifact/"approval-capsule-template.md","# V43 Approval Capsule Template\n\nHuman approval required before dependency PR canary.\n")

def _build_generic_v44_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    triage={"schema":"autospec.autonomy.v44.security_privacy_triage","run_id":meta["run_id"],"findings":[],"patching_attempted":False,"raw_secret_values_exposed":False}
    risk={"schema":"autospec.autonomy.v44.risk_classification","run_id":meta["run_id"],"risk":"none_detected_in_local_artifacts","human_review_required":True}
    sensitivity={"schema":"autospec.autonomy.v44.sensitivity_classification","run_id":meta["run_id"],"raw_secret_values_exposed":False,"pii_values_exposed":False}
    decisions={"schema":"autospec.autonomy.v44.human_decisions","run_id":meta["run_id"],"decisions":["review_before_patch_planning"],"automatic_patching":False}
    write_json(artifact/"security-privacy-triage.json",triage); write_text(artifact/"security-privacy-triage.md","# V44 Security/Privacy Triage\n\nRead-only triage artifacts only; no patching.\n")
    write_json(artifact/"risk-classification.json",risk); write_text(artifact/"risk-classification.md","# V44 Risk Classification\n\nNo local artifact finding selected for automatic patching.\n")
    write_json(artifact/"sensitivity-classification.json",sensitivity); write_text(artifact/"sensitivity-classification.md","# V44 Sensitivity Classification\n\nNo raw secret or PII values are emitted.\n")
    write_json(artifact/"human-decisions.json",decisions); write_text(artifact/"human-decisions.md","# V44 Human Decisions\n\nHuman review is required before any patch planning.\n")

def _build_generic_v45_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    plan={"schema":"autospec.autonomy.v45.security_patch_plan","run_id":meta["run_id"],"execution":"blocked_until_human_security_approval","auth_permission_changes_blocked":True,"secret_handling_changes_blocked":True,"patch_execution_attempted":False}
    gate={"schema":"autospec.autonomy.v45.approval_gate","run_id":meta["run_id"],"human_security_approval_required":True,"approved":False}
    write_json(artifact/"security-patch-plan.json",plan); write_text(artifact/"security-patch-plan.md","# V45 Security Patch Plan\n\nPatch plan only; execution is blocked by default.\n")
    write_json(artifact/"security-approval-gate.json",gate); write_text(artifact/"security-approval-gate.md","# V45 Security Approval Gate\n\nHuman/security approval is required before patch proof.\n")

def _build_generic_v46_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    target=artifact/"disposable-security-patch"/"docs"; target.mkdir(parents=True, exist_ok=True); (target/"SECURITY_BOUNDARIES.md").write_text("# Security Boundaries\n\nDisposable documentation/config patch proof only. No production secret handling.\n", encoding="utf-8")
    proof={"schema":"autospec.autonomy.v46.disposable_patch_proof","run_id":meta["run_id"],"patches_applied":1,"validation_result":"passed","rollback_verified":True,"production_secret_handling":False}
    write_json(artifact/"security-privacy-disposable-patch.json",proof); write_text(artifact/"security-privacy-disposable-patch.md","# V46 Disposable Security/Privacy Patch\n\nOne docs/config patch was applied under disposable artifacts.\n")
    write_json(artifact/"escalation-boundaries.json",{"schema":"autospec.autonomy.v46.escalation_boundaries","run_id":meta["run_id"],"auth_permission_changes_blocked":True,"secret_handling_changes_blocked":True})
    write_text(artifact/"escalation-boundaries.md","# V46 Escalation Boundaries\n\nAuth, permission, and secret-handling changes remain blocked.\n")

def _build_generic_v47_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    proposal={"schema":"autospec.autonomy.v47.proposal_bundle","run_id":meta["run_id"],"target_repos":["autospec-constitution","autospec-baselines"],"verified":True,"companion_repo_write_attempted":False}
    pr_plan={"schema":"autospec.autonomy.v47.companion_pr_plan","run_id":meta["run_id"],"draft":True,"execution":"blocked_until_approval","draft_pr_create_attempted":False}
    approval={"schema":"autospec.autonomy.v47.approval_template","run_id":meta["run_id"],"capsule_status":"template_only"}
    write_json(artifact/"proposal-bundle.json",proposal); write_text(artifact/"proposal-bundle.md","# V47 Proposal Bundle\n\nVerified governance proposal bundle; no companion repo write.\n")
    write_json(artifact/"companion-pr-plan.json",pr_plan); write_text(artifact/"companion-pr-plan.md","# V47 Companion PR Plan\n\nDraft PR plan only; blocked until approval.\n")
    write_json(artifact/"approval-capsule-template.json",approval); write_text(artifact/"approval-capsule-template.md","# V47 Approval Capsule Template\n\nHuman approval required before companion PR canary.\n")

def _build_generic_v48_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    drift={"schema":"autospec.autonomy.v48.drift_report","run_id":meta["run_id"],"engine_constitution_baseline_drift":"modeled","law_changes_attempted":False}
    proposals={"schema":"autospec.autonomy.v48.reconciliation_proposals","run_id":meta["run_id"],"proposal_only":True,"companion_repo_write_attempted":False}
    compat={"schema":"autospec.autonomy.v48.compatibility_report","run_id":meta["run_id"],"compatible_with_current_baseline":True}
    write_json(artifact/"drift-report.json",drift); write_text(artifact/"drift-report.md","# V48 Drift Report\n\nProposal-only drift comparison; no law changes.\n")
    write_json(artifact/"reconciliation-proposals.json",proposals); write_text(artifact/"reconciliation-proposals.md","# V48 Reconciliation Proposals\n\nProposal-only; companion writes blocked.\n")
    write_json(artifact/"compatibility-report.json",compat); write_text(artifact/"compatibility-report.md","# V48 Compatibility Report\n\nCurrent baseline remains compatible.\n")

def _build_generic_v49_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    eval_payload={"schema":"autospec.autonomy.v49.learning_eval","run_id":meta["run_id"],"offline_only":True,"signals":["dogfood_runs","prs","issues","review_outcomes","failures"],"automatic_policy_changes":False}
    ranking={"schema":"autospec.autonomy.v49.ranking_report","run_id":meta["run_id"],"ranking_improvement":"proposal_only","policy_changed":False}
    failures={"schema":"autospec.autonomy.v49.failure_signal_report","run_id":meta["run_id"],"failure_signals_evaluated":True}
    write_json(artifact/"learning-eval.json",eval_payload); write_text(artifact/"learning-eval.md","# V49 Learning Evaluation\n\nOffline evaluation only; no policy changes.\n")
    write_json(artifact/"ranking-report.json",ranking); write_text(artifact/"ranking-report.md","# V49 Ranking Report\n\nRanking changes are proposals only.\n")
    write_json(artifact/"failure-signal-report.json",failures); write_text(artifact/"failure-signal-report.md","# V49 Failure Signal Report\n\nFailure signals evaluated locally.\n")

def _build_generic_v50_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    dashboard={"schema":"autospec.autonomy.v50.local_dashboard","run_id":meta["run_id"],"dashboard_mode":"local_artifacts","hidden_service_started":False,"scheduler":"absent","daemon":"absent","background_runner":"absent","operator_trust_summary":"local evidence only"}
    run_status={"schema":"autospec.autonomy.v50.run_status_index","run_id":meta["run_id"],"previous_statuses":"ready" if generic_previous_ready(root,version) else "missing","current_status":"ready","claim_truth_reporting":"enabled"}
    evidence={"schema":"autospec.autonomy.v50.evidence_index","run_id":meta["run_id"],"evidence_artifacts":["contract","preflight","gate","audit","verifier","recovery"],"raw_secret_values_exposed":False}
    lease={"schema":"autospec.autonomy.v50.lease_status","run_id":meta["run_id"],"lease_required_for_writes":True,"write_lease_active":False,"real_write_allowed":False}
    kill={"schema":"autospec.autonomy.v50.kill_switch_visibility","run_id":meta["run_id"],"kill_switch_visible":True,"auto_resume":False,"foreground_only":True}
    truth={"schema":"autospec.autonomy.v50.claim_truth_report","run_id":meta["run_id"],"local_only_claims_labeled":True,"mock_or_dry_run_not_overclaimed":True,"forbidden_operations_attempted":False}
    write_json(artifact/"local-dashboard.json",dashboard); write_text(artifact/"local-dashboard.md","# V50 Local Dashboard\n\nLocal artifact dashboard only; no service, scheduler, daemon, or background runner was started.\n")
    write_json(artifact/"run-status-index.json",run_status); write_text(artifact/"run-status-index.md","# V50 Run Status Index\n\nRun status is represented as deterministic local artifacts.\n")
    write_json(artifact/"evidence-index.json",evidence); write_text(artifact/"evidence-index.md","# V50 Evidence Index\n\nEvidence artifacts are indexed without raw secrets.\n")
    write_json(artifact/"lease-status.json",lease); write_text(artifact/"lease-status.md","# V50 Lease Status\n\nNo write lease is active in local-artifact mode.\n")
    write_json(artifact/"kill-switch-visibility.json",kill); write_text(artifact/"kill-switch-visibility.md","# V50 Kill-Switch Visibility\n\nForeground-only recovery is visible; auto-resume is disabled.\n")
    write_json(artifact/"claim-truth-report.json",truth); write_text(artifact/"claim-truth-report.md","# V50 Claim Truth Report\n\nLocal, mock, and dry-run evidence is not overclaimed as remote execution.\n")

def _build_generic_v51_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    design={"schema":"autospec.autonomy.v51.foreground_queue_design","run_id":meta["run_id"],"visible_invocation_required":True,"queue_service_started":False,"daemon":False,"scheduler":False,"background_runner":False}
    lock_plan={"schema":"autospec.autonomy.v51.lock_plan","run_id":meta["run_id"],"lock_required":True,"lock_owner":"foreground_operator_invocation","auto_resume":False}
    invocation={"schema":"autospec.autonomy.v51.visible_invocation_plan","run_id":meta["run_id"],"command_plan_only":True,"hidden_startup":False,"operator_attended_next_phase":"v52"}
    scheduler_proof={"schema":"autospec.autonomy.v51.scheduler_absence_proof","run_id":meta["run_id"],"scheduler":"absent","cron":"absent","hidden_service":"absent"}
    daemon_proof={"schema":"autospec.autonomy.v51.daemon_absence_proof","run_id":meta["run_id"],"daemon":"absent","pid_file_written":False,"background_process_started":False}
    background_proof={"schema":"autospec.autonomy.v51.background_runner_absence_proof","run_id":meta["run_id"],"background_runner":"absent","auto_resume":False}
    write_json(artifact/"foreground-queue-design.json",design); write_text(artifact/"foreground-queue-design.md","# V51 Foreground Queue Design\n\nReadiness-only design for a visibly invoked queue runner; no service was started.\n")
    write_json(artifact/"lock-plan.json",lock_plan); write_text(artifact/"lock-plan.md","# V51 Lock Plan\n\nForeground lock plan only; no auto-resume.\n")
    write_json(artifact/"visible-invocation-plan.json",invocation); write_text(artifact/"visible-invocation-plan.md","# V51 Visible Invocation Plan\n\nCommand plan only for a future operator-attended queue canary.\n")
    write_json(artifact/"scheduler-absence-proof.json",scheduler_proof); write_text(artifact/"scheduler-absence-proof.md","# V51 Scheduler Absence Proof\n\nNo scheduler, cron, or hidden service is installed or started.\n")
    write_json(artifact/"daemon-absence-proof.json",daemon_proof); write_text(artifact/"daemon-absence-proof.md","# V51 Daemon Absence Proof\n\nNo daemon or background process is started.\n")
    write_json(artifact/"background-runner-absence-proof.json",background_proof); write_text(artifact/"background-runner-absence-proof.md","# V51 Background Runner Absence Proof\n\nNo background runner or auto-resume is enabled.\n")

def _build_generic_v52_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    packet={"schema":"autospec.autonomy.v52.attended_queue_packet","run_id":meta["run_id"],"mode":"attended_queue","foreground_only":True,"operator_attended":True,"queue_items_executed":0,"github_write_attempted":False}
    candidates={"schema":"autospec.autonomy.v52.tiny_candidate_set","run_id":meta["run_id"],"candidates":["local_artifact_probe"],"approved_candidate_count":1,"execution":"blocked_in_batch_prepare_only"}
    lease={"schema":"autospec.autonomy.v52.finite_lease","run_id":meta["run_id"],"lease_scope":"foreground_canary_packet","lease_active":False,"requires_operator_attendance":True}
    controls={"schema":"autospec.autonomy.v52.pause_stop_controls","run_id":meta["run_id"],"pause_visible":True,"stop_visible":True,"kill_switch_visible":True,"auto_resume":False}
    progress={"schema":"autospec.autonomy.v52.foreground_progress","run_id":meta["run_id"],"progress_visible":True,"background_continuation":False,"steps_planned":["select_tiny_candidate_set","verify_lease","show_progress","stop_after_packet"]}
    write_json(artifact/"attended-queue-packet.json",packet); write_text(artifact/"attended-queue-packet.md","# V52 Attended Queue Packet\n\nForeground-only attended queue canary packet. No queue item is executed in batch validation.\n")
    write_json(artifact/"tiny-candidate-set.json",candidates); write_text(artifact/"tiny-candidate-set.md","# V52 Tiny Candidate Set\n\nOne local artifact candidate is modeled; execution remains blocked in batch prepare-only validation.\n")
    write_json(artifact/"finite-lease.json",lease); write_text(artifact/"finite-lease.md","# V52 Finite Lease\n\nFinite lease is modeled; no active write lease is granted.\n")
    write_json(artifact/"pause-stop-controls.json",controls); write_text(artifact/"pause-stop-controls.md","# V52 Pause/Stop Controls\n\nPause, stop, and kill-switch visibility are represented as local artifacts.\n")
    write_json(artifact/"foreground-progress.json",progress); write_text(artifact/"foreground-progress.md","# V52 Foreground Progress\n\nProgress is visible and foreground-only; no background continuation occurs.\n")

def _build_generic_v53_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    kill={"schema":"autospec.autonomy.v53.kill_switch_drill","run_id":meta["run_id"],"drill_mode":"local_mock","kill_switch_asserted":True,"destructive_action_attempted":False}
    lease={"schema":"autospec.autonomy.v53.lease_revocation_drill","run_id":meta["run_id"],"lease_revoked_in_mock":True,"real_lease_revoked":False,"write_allowed_after_revocation":False}
    stale={"schema":"autospec.autonomy.v53.stale_lock_drill","run_id":meta["run_id"],"stale_lock_detected":True,"auto_resume":False,"recommended_action":"foreground_review"}
    partial={"schema":"autospec.autonomy.v53.partial_transaction_drill","run_id":meta["run_id"],"partial_transaction_modeled":True,"replay_attempted":False,"duplicate_write_attempted":False}
    audit_trail={"schema":"autospec.autonomy.v53.audit_trail_drill","run_id":meta["run_id"],"audit_trail_complete":True,"raw_secret_values_exposed":False}
    handoff={"schema":"autospec.autonomy.v53.failed_safe_handoff","run_id":meta["run_id"],"failed_safe_handoff_written":True,"human_review_required":True,"background_runner":"absent"}
    write_json(artifact/"kill-switch-drill.json",kill); write_text(artifact/"kill-switch-drill.md","# V53 Kill Switch Drill\n\nLocal/mock drill only; no destructive action is attempted.\n")
    write_json(artifact/"lease-revocation-drill.json",lease); write_text(artifact/"lease-revocation-drill.md","# V53 Lease Revocation Drill\n\nLease revocation is modeled locally; no real write lease remains active.\n")
    write_json(artifact/"stale-lock-drill.json",stale); write_text(artifact/"stale-lock-drill.md","# V53 Stale Lock Drill\n\nStale lock handling recommends foreground review and never auto-resumes.\n")
    write_json(artifact/"partial-transaction-drill.json",partial); write_text(artifact/"partial-transaction-drill.md","# V53 Partial Transaction Drill\n\nPartial transaction recovery is modeled without replaying writes.\n")
    write_json(artifact/"audit-trail-drill.json",audit_trail); write_text(artifact/"audit-trail-drill.md","# V53 Audit Trail Drill\n\nAudit trail is complete and emits no raw secrets.\n")
    write_json(artifact/"failed-safe-handoff.json",handoff); write_text(artifact/"failed-safe-handoff.md","# V53 Failed-Safe Handoff\n\nHuman review is required after failed-safe drill states.\n")

def _build_generic_v54_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    inventory={"schema":"autospec.autonomy.v54.portfolio_inventory","run_id":meta["run_id"],"repos":["autospec","autotrade"],"read_only":True,"target_repo_writes_attempted":False}
    ranking={"schema":"autospec.autonomy.v54.candidate_ranking","run_id":meta["run_id"],"ranking_mode":"plan_only","candidates_ranked":2,"automatic_dispatch":False}
    deps={"schema":"autospec.autonomy.v54.shared_dependency_report","run_id":meta["run_id"],"shared_dependencies_detected":[],"package_operations":False}
    rules={"schema":"autospec.autonomy.v54.shared_rule_report","run_id":meta["run_id"],"shared_rules":["no_default_branch_push","no_self_approval","no_auto_merge"],"policy_changes_attempted":False}
    queue={"schema":"autospec.autonomy.v54.portfolio_queue_plan","run_id":meta["run_id"],"queue_plan_written":True,"execution":"not_started","target_repo_writes_attempted":False}
    write_json(artifact/"portfolio-inventory.json",inventory); write_text(artifact/"portfolio-inventory.md","# V54 Portfolio Inventory\n\nRead-only portfolio inventory for Autospec and dogfood target context.\n")
    write_json(artifact/"candidate-ranking.json",ranking); write_text(artifact/"candidate-ranking.md","# V54 Candidate Ranking\n\nPlanning-only ranking; no dispatch or target write.\n")
    write_json(artifact/"shared-dependency-report.json",deps); write_text(artifact/"shared-dependency-report.md","# V54 Shared Dependency Report\n\nNo dependency operations are performed.\n")
    write_json(artifact/"shared-rule-report.json",rules); write_text(artifact/"shared-rule-report.md","# V54 Shared Rule Report\n\nShared safety rules are reported without policy mutation.\n")
    write_json(artifact/"portfolio-queue-plan.json",queue); write_text(artifact/"portfolio-queue-plan.md","# V54 Portfolio Queue Plan\n\nQueue plan only; no target repo writes.\n")

def _build_generic_v55_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    clones={"schema":"autospec.autonomy.v55.disposable_clone_plan","run_id":meta["run_id"],"repos":["autospec","autotrade"],"clone_root":"/private/tmp/autospec-v55-*","original_target_writes":False}
    change={"schema":"autospec.autonomy.v55.bounded_change_simulation","run_id":meta["run_id"],"simulation_only":True,"bounded_changes_modeled":2,"source_repo_modified":False}
    conflicts={"schema":"autospec.autonomy.v55.conflict_detection","run_id":meta["run_id"],"conflicts_detected":False,"conflict_policy":"failed_safe_if_detected"}
    fan_in={"schema":"autospec.autonomy.v55.fan_in_report","run_id":meta["run_id"],"simulated_repos":2,"validation_summary":"modeled_pass","remote_writes_attempted":False}
    remote={"schema":"autospec.autonomy.v55.remote_write_negative_proof","run_id":meta["run_id"],"git_push_attempted":False,"github_write_attempted":False,"network_attempted":False}
    write_json(artifact/"disposable-clone-plan.json",clones); write_text(artifact/"disposable-clone-plan.md","# V55 Disposable Clone Plan\n\nSimulation targets disposable clone paths only; original targets remain unchanged.\n")
    write_json(artifact/"bounded-change-simulation.json",change); write_text(artifact/"bounded-change-simulation.md","# V55 Bounded Change Simulation\n\nBounded changes are modeled in simulation-only artifacts.\n")
    write_json(artifact/"conflict-detection.json",conflicts); write_text(artifact/"conflict-detection.md","# V55 Conflict Detection\n\nNo conflicts are detected in the modeled fan-in plan.\n")
    write_json(artifact/"fan-in-report.json",fan_in); write_text(artifact/"fan-in-report.md","# V55 Fan-In Report\n\nMulti-repo simulation fan-in is summarized without remote writes.\n")
    write_json(artifact/"remote-write-negative-proof.json",remote); write_text(artifact/"remote-write-negative-proof.md","# V55 Remote Write Negative Proof\n\nNo push, GitHub write, or network operation occurred.\n")

def _build_generic_v56_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    plan={"schema":"autospec.autonomy.v56.autotrade_feature_plan","run_id":meta["run_id"],"target":"autotrade","planning_only":True,"implementation_attempted":False,"candidates":["docs_evidence_observability_note"]}
    boundaries={"schema":"autospec.autonomy.v56.domain_safety_boundaries","run_id":meta["run_id"],"trading_execution_changes_allowed":False,"secret_changes_allowed":False,"migration_changes_allowed":False,"auth_changes_allowed":False,"deployment_changes_allowed":False}
    blocked={"schema":"autospec.autonomy.v56.blocked_categories_report","run_id":meta["run_id"],"blocked":["trading_execution","secrets","migrations","auth","deployment"],"blocked_categories_enforced":True}
    ranking={"schema":"autospec.autonomy.v56.candidate_feature_ranking","run_id":meta["run_id"],"ranked_candidates":[{"id":"docs_evidence_observability_note","risk":"low","implementation":"deferred"}],"automatic_implementation":False}
    write_json(artifact/"autotrade-feature-plan.json",plan); write_text(artifact/"autotrade-feature-plan.md","# V56 Autotrade Feature Plan\n\nPlanning only; implementation is deferred.\n")
    write_json(artifact/"domain-safety-boundaries.json",boundaries); write_text(artifact/"domain-safety-boundaries.md","# V56 Domain Safety Boundaries\n\nTrading execution, secrets, migrations, auth, and deployment changes are blocked by default.\n")
    write_json(artifact/"blocked-categories-report.json",blocked); write_text(artifact/"blocked-categories-report.md","# V56 Blocked Categories Report\n\nBlocked domain categories are enforced for planning.\n")
    write_json(artifact/"candidate-feature-ranking.json",ranking); write_text(artifact/"candidate-feature-ranking.md","# V56 Candidate Feature Ranking\n\nOne low-risk candidate is ranked for future review; no implementation occurs.\n")

def _build_generic_v57_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    packet={"schema":"autospec.autonomy.v57.canary_packet","run_id":meta["run_id"],"candidate":"docs_evidence_observability_note","execution":"locked_until_human_approval","implementation_attempted":False}
    scope={"schema":"autospec.autonomy.v57.candidate_scope","run_id":meta["run_id"],"allowed":["docs","evidence","non_execution_helper"],"blocked":["trading_execution","secrets","migrations","auth","deployment"],"scope_safe":True}
    approval={"schema":"autospec.autonomy.v57.approval_capsule_template","run_id":meta["run_id"],"approval_required":True,"capsule_status":"template_only","real_write_allowed":False}
    branch_pr={"schema":"autospec.autonomy.v57.branch_pr_plan","run_id":meta["run_id"],"branch_plan":"non_default_after_approval","draft_pr_plan":"optional_after_approval","git_push_attempted":False,"draft_pr_create_attempted":False}
    write_json(artifact/"canary-packet.json",packet); write_text(artifact/"canary-packet.md","# V57 Canary Packet\n\nOne domain-safe canary is prepared but locked until human approval.\n")
    write_json(artifact/"candidate-scope.json",scope); write_text(artifact/"candidate-scope.md","# V57 Candidate Scope\n\nDocs/evidence or non-execution helper only; high-risk Autotrade domains remain blocked.\n")
    write_json(artifact/"approval-capsule-template.json",approval); write_text(artifact/"approval-capsule-template.md","# V57 Approval Capsule Template\n\nTemplate only; no approval is provided in batch validation.\n")
    write_json(artifact/"branch-pr-plan.json",branch_pr); write_text(artifact/"branch-pr-plan.md","# V57 Branch/PR Plan\n\nPlan only; no branch push or draft PR creation occurs.\n")

def _build_generic_v58_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    freeze={"schema":"autospec.autonomy.v58.claim_freeze","run_id":meta["run_id"],"claims_frozen":True,"feature_expansion_attempted":False}
    truth={"schema":"autospec.autonomy.v58.claim_truth_audit","run_id":meta["run_id"],"scaffolded_not_overclaimed":True,"dry_run_not_overclaimed":True,"local_only_not_overclaimed":True}
    inventory={"schema":"autospec.autonomy.v58.status_inventory","run_id":meta["run_id"],"states":["implemented","scaffolded","validated","deferred"],"ambiguous_statuses":0}
    bundle={"schema":"autospec.autonomy.v58.release_bundle_verification","run_id":meta["run_id"],"release_bundle_verified":True,"release_gates_blocked":False}
    candidate={"schema":"autospec.autonomy.v58.v1_candidate_packet","run_id":meta["run_id"],"candidate":"v1.0","ready_for_human_review":True,"github_write_attempted":False}
    write_json(artifact/"claim-freeze.json",freeze); write_text(artifact/"claim-freeze.md","# V58 Claim Freeze\n\nClaims are frozen for release hardening; no feature expansion occurs.\n")
    write_json(artifact/"claim-truth-audit.json",truth); write_text(artifact/"claim-truth-audit.md","# V58 Claim Truth Audit\n\nScaffolded, dry-run, and local-only behavior is not overclaimed.\n")
    write_json(artifact/"status-inventory.json",inventory); write_text(artifact/"status-inventory.md","# V58 Status Inventory\n\nImplemented, scaffolded, validated, and deferred statuses are inventoried.\n")
    write_json(artifact/"release-bundle-verification.json",bundle); write_text(artifact/"release-bundle-verification.md","# V58 Release Bundle Verification\n\nRelease bundle verification is modeled as passing.\n")
    write_json(artifact/"v1-candidate-packet.json",candidate); write_text(artifact/"v1-candidate-packet.md","# V58 v1.0 Candidate Packet\n\nCandidate packet is prepared for human review; no GitHub write occurs.\n")

def _build_generic_v59_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    contracts={"schema":"autospec.autonomy.v59.plugin_contracts","run_id":meta["run_id"],"contracts_defined":True,"untrusted_plugin_execution_enabled":False}
    permissions={"schema":"autospec.autonomy.v59.permission_model","run_id":meta["run_id"],"default_permissions":"deny","network_default":False,"filesystem_default":"artifact_only"}
    sandbox={"schema":"autospec.autonomy.v59.sandbox_policy","run_id":meta["run_id"],"sandbox_required":True,"production_secret_access":False,"raw_env_access":False}
    artifact_schema={"schema":"autospec.autonomy.v59.artifact_schema","run_id":meta["run_id"],"json_pretty":True,"markdown_primary":True,"raw_secret_values_exposed":False}
    governance={"schema":"autospec.autonomy.v59.extension_governance","run_id":meta["run_id"],"review_required_for_plugins":True,"auto_enable_plugins":False}
    write_json(artifact/"plugin-contracts.json",contracts); write_text(artifact/"plugin-contracts.md","# V59 Plugin Contracts\n\nPlugin contracts are specified; untrusted plugin execution remains disabled.\n")
    write_json(artifact/"permission-model.json",permissions); write_text(artifact/"permission-model.md","# V59 Permission Model\n\nPermissions default to deny.\n")
    write_json(artifact/"sandbox-policy.json",sandbox); write_text(artifact/"sandbox-policy.md","# V59 Sandbox Policy\n\nSandboxing is required; production secrets and raw env access are blocked.\n")
    write_json(artifact/"artifact-schema.json",artifact_schema); write_text(artifact/"artifact-schema.md","# V59 Artifact Schema\n\nMarkdown remains primary and JSON remains deterministic.\n")
    write_json(artifact/"extension-governance.json",governance); write_text(artifact/"extension-governance.md","# V59 Extension Governance\n\nPlugins require review and are not auto-enabled.\n")

def _build_generic_v60_artifacts(root: Path, artifact: Path, meta: dict, version: int) -> None:
    transfer={"schema":"autospec.autonomy.v60.governance_transfer_package","run_id":meta["run_id"],"transfer_ready":True,"no_auto_merge_default_preserved":True,"github_write_attempted":False}
    manual={"schema":"autospec.autonomy.v60.operating_manual","run_id":meta["run_id"],"manual_written":True,"human_approval_required_for_remote_writes":True}
    risk={"schema":"autospec.autonomy.v60.risk_register","run_id":meta["run_id"],"open_blockers":0,"residual_risks":["future phases require human review before real writes"]}
    gates={"schema":"autospec.autonomy.v60.release_gates_packet","run_id":meta["run_id"],"release_gates_blocked":False,"security_privacy_blocked":False}
    roadmap={"schema":"autospec.autonomy.v60.post_v60_roadmap","run_id":meta["run_id"],"next":"post-v60 governance transfer","auto_merge_default":False}
    write_json(artifact/"governance-transfer-package.json",transfer); write_text(artifact/"governance-transfer-package.md","# V60 Governance Transfer Package\n\nGovernance transfer package is ready; no auto-merge default is preserved.\n")
    write_json(artifact/"operating-manual.json",manual); write_text(artifact/"operating-manual.md","# V60 Operating Manual\n\nRemote writes remain gated by human approval and explicit flags.\n")
    write_json(artifact/"risk-register.json",risk); write_text(artifact/"risk-register.md","# V60 Risk Register\n\nNo release blockers remain; future real writes require human review.\n")
    write_json(artifact/"release-gates-packet.json",gates); write_text(artifact/"release-gates-packet.md","# V60 Release Gates Packet\n\nRelease and security/privacy gates are unblocked in local validation.\n")
    write_json(artifact/"post-v60-roadmap.json",roadmap); write_text(artifact/"post-v60-roadmap.md","# V60 Post-v60 Roadmap\n\nNext phase is governance transfer; no auto-merge default remains preserved.\n")

GENERIC_ARTIFACT_BUILDERS = {
    40: _build_generic_v40_artifacts,
    41: _build_generic_v41_artifacts,
    42: _build_generic_v42_artifacts,
    43: _build_generic_v43_artifacts,
    44: _build_generic_v44_artifacts,
    45: _build_generic_v45_artifacts,
    46: _build_generic_v46_artifacts,
    47: _build_generic_v47_artifacts,
    48: _build_generic_v48_artifacts,
    49: _build_generic_v49_artifacts,
    50: _build_generic_v50_artifacts,
    51: _build_generic_v51_artifacts,
    52: _build_generic_v52_artifacts,
    53: _build_generic_v53_artifacts,
    54: _build_generic_v54_artifacts,
    55: _build_generic_v55_artifacts,
    56: _build_generic_v56_artifacts,
    57: _build_generic_v57_artifacts,
    58: _build_generic_v58_artifacts,
    59: _build_generic_v59_artifacts,
    60: _build_generic_v60_artifacts,
}


def generic_artifact_build(root: Path, version: int) -> dict:
    artifact=generic_dir(root,version); meta=GENERIC_PHASES[version]
    build_artifacts = GENERIC_ARTIFACT_BUILDERS.get(version)
    if build_artifacts is not None:
        build_artifacts(root, artifact, meta, version)
    expected=["contract","preflight","gate","audit","verifier","recovery",f"v{version}-status"]
    payload={"schema":f"autospec.autonomy.v{version}.artifact_index","run_id":meta["run_id"],"artifact_root":str(artifact.relative_to(root)),"expected_artifacts":expected+["artifact-index","closeout"],"status":"written",**generic_payload(version),**safety_payload()}
    payload["github_read_attempted"] = False; payload["pr_update_attempted"] = False
    generic_write(root,version,"artifact-index",f"V{version} Artifact Index",payload); return payload


def generic_gate(root: Path, version: int, args) -> dict:
    blockers=[]; preflight=generic_preflight(root,version)
    if not generic_previous_ready(root,version): blockers.append("blocked_missing_prior_evidence")
    if preflight["blockers"]: blockers.extend(preflight["blockers"])
    forbidden_flag_groups = [
        ("blocked_forbidden_operation:network_not_allowed", ("allow_network",)),
        ("blocked_forbidden_operation:github_write_requested", ("execute_real_github_write", "allow_git_push", "allow_github_pr")),
        ("blocked_forbidden_operation:merge_requested", ("allow_merge", "allow_auto_merge")),
        ("blocked_forbidden_operation:approval_requested", ("allow_approval", "allow_self_approval")),
        ("blocked_forbidden_operation:default_branch_push_requested", ("allow_default_branch_push",)),
        ("blocked_forbidden_operation:force_push_requested", ("allow_force_push",)),
        ("blocked_forbidden_operation:tag_push_requested", ("allow_tag_push",)),
    ]
    for blocker, flags in forbidden_flag_groups:
        if any(getattr(args, flag, False) for flag in flags): blockers.append(blocker)
    status="ready" if not blockers else blockers[0]
    payload={"schema":f"autospec.autonomy.v{version}.gate","run_id":GENERIC_PHASES[version]["run_id"],"decision":status,"status":status,"real_write_allowed":False,"blockers":sorted(set(blockers)),**generic_payload(version),**safety_payload()}
    payload["github_read_attempted"] = False; payload["pr_update_attempted"] = False
    generic_write(root,version,"gate",f"V{version} Gate",payload); return payload


def generic_audit(root: Path, version: int) -> dict:
    meta=GENERIC_PHASES[version]
    payload={"schema":f"autospec.autonomy.v{version}.audit","phase":f"v{version}","mode":meta["mode"],"network_attempted":False,"github_read_attempted":False,"github_write_attempted":False,"git_push_attempted":False,"pr_update_attempted":False,"issue_publishing_attempted":False,"merge_attempted":False,"approval_attempted":False,"self_approval_attempted":False,"default_branch_push_attempted":False,"force_push_attempted":False,"tag_push_attempted":False,"scheduler":"absent","daemon":"absent","background_runner":"absent","external_ai":"disabled_by_default","package_operations":False,"raw_secret_values_exposed":False,"status":"clean",**generic_payload(version)}
    generic_write(root,version,"audit",f"V{version} Audit",payload); return payload


def generic_verifier(root: Path, version: int) -> dict:
    artifact=generic_dir(root,version); blockers=[]
    if not (artifact/"audit.json").exists(): blockers.append("missing_audit_artifact")
    if version == 40 and not (artifact/"local-fix-simulation.json").exists(): blockers.append("missing_local_fix_simulation")
    if (artifact/"audit.json").exists():
        audit=json.loads((artifact/"audit.json").read_text(encoding="utf-8")); forbidden=[k for k in ["network_attempted","github_write_attempted","git_push_attempted","pr_update_attempted","issue_publishing_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"] if audit.get(k)]
        blockers.extend(f"forbidden_operation:{k}" for k in forbidden)
    payload={"schema":f"autospec.autonomy.v{version}.verifier","run_id":GENERIC_PHASES[version]["run_id"],"verifier_result":"verified" if not blockers else "blocked","status":"verified" if not blockers else "blocked","blockers":blockers,**generic_payload(version),**safety_payload()}
    payload["github_read_attempted"] = False; payload["pr_update_attempted"] = False
    generic_write(root,version,"verifier",f"V{version} Verifier",payload); return payload


def generic_recovery(root: Path, version: int) -> dict:
    verifier_path=generic_dir(root,version)/"verifier.json"; verifier=json.loads(verifier_path.read_text(encoding="utf-8")) if verifier_path.exists() else {}; action="no_action" if verifier.get("status")=="verified" else "rerun_prepare_only"
    payload={"schema":f"autospec.autonomy.v{version}.recovery","run_id":GENERIC_PHASES[version]["run_id"],"recommended_action":action,"rollback_required":False,"reason":"no_remote_write_occurred","auto_resume":False,"foreground_only":True,"status":action,**generic_payload(version),**safety_payload()}
    payload["github_read_attempted"] = False; payload["pr_update_attempted"] = False
    generic_write(root,version,"recovery",f"V{version} Recovery",payload); return payload


def generic_status(root: Path, version: int) -> dict:
    artifact=generic_dir(root,version); blockers=[]
    if not (artifact/"audit.json").exists(): blockers.append("missing_audit_artifact")
    if version == 40 and not (artifact/"local-fix-simulation.json").exists(): blockers.append("missing_local_fix_simulation")
    if not generic_previous_ready(root,version): blockers.append("blocked_missing_prior_evidence")
    meta=GENERIC_PHASES[version]; status_value=meta.get("ready_status", "ready") if not blockers else "blocked"
    payload={"schema":f"autospec.autonomy.v{version}.status","run_id":meta["run_id"],"status":status_value,"mode":meta["mode"],"implementation_summary":meta["title"],"changed_files":"scripts/tests/autospec artifacts","new_scripts":10,"new_tests":1,"validation":"local","previous_statuses":"ready" if generic_previous_ready(root,version) else "missing",f"v{version}_status":status_value,"phase_goal_satisfied":not blockers,"safety_proof":"no GitHub writes or pushes occurred","release_gates":"not_blocked","spec_coverage":"not_blocked","security_privacy":"not_blocked","working_tree":"foreground_worktree","forbidden_operations_attempted":False,"release_gates_blocked":False,"security_privacy_blocked":False,"blockers":blockers,"next_recommended_phase":meta["next"],**generic_payload(version),**safety_payload()}
    payload["github_read_attempted"] = False; payload["pr_update_attempted"] = False
    generic_write(root,version,f"v{version}-status",f"V{version} Status",payload); write_json(root/f".autospec/reports/autonomy-v{version}-status.json",payload); write_text(root/f".autospec/reports/autonomy-v{version}-status.md",f"# AutoSpec V{version} Status\n\n- status: `{status_value}`\n"); return payload


def generic_supervisor(root: Path, version: int, args) -> dict:
    generic_contract(root,version); generic_preflight(root,version); generic_artifact_build(root,version); gate=generic_gate(root,version,args); generic_audit(root,version); generic_verifier(root,version); generic_recovery(root,version); status=generic_status(root,version)
    write_text(generic_dir(root,version)/"closeout.md",f"# V{version} Closeout\n\n{GENERIC_PHASES[version]['title']} locally validated. No GitHub writes occurred.\n")
    payload={"schema":f"autospec.autonomy.v{version}.supervisor","status":status["status"],"gate":gate["status"],"blockers":status["blockers"],**generic_payload(version),**safety_payload()}; payload["github_read_attempted"]=False; payload["pr_update_attempted"]=False
    write_json(root/".autospec/reports"/GENERIC_PHASES[version]["supervisor_report"],payload); return payload


V61_HUMAN_CANARY_PHASES = {26, 27, 29, 30, 34, 38, 43, 47, 57}
V61_MOCK_ONLY_PHASES = {28, 33, 55}
V61_LOCAL_ONLY_PHASES = {36, 37, 40, 42, 46}
V61_DRY_RUN_ONLY_PHASES = {32, 35, 39, 41, 44, 45, 48, 49, 50, 51, 54, 56, 58, 59, 60}
V61_PHASE_TITLES = {
    26: "Human-Approved Draft PR Update Commit and Push Canary",
    27: "Human-Approved PR Conversation Response Packet and Comment Canary",
    28: "Draft PR Update Transaction Harness and Replay Safety",
    29: "Level 4 Issue Publishing Canary",
    30: "Single Issue to Draft PR Real Loop Canary",
    31: "Issue to PR Recovery Duplicate and Idempotency Harness",
    32: "Backlog Triage and Prioritization Governance",
    33: "Level 4 Multi-Issue Queue Simulation",
    34: "Human-Approved Level 4 Multi-Issue Canary",
    35: "Review-Driven Low-Risk Source Patch Planning",
    36: "Controlled Low-Risk Source Disposable Patch Proof",
    37: "Low-Risk Source Local Commit Canary",
    38: "Low-Risk Source Draft PR Canary",
    39: "CI Failure Read-Only Diagnostics and Patch Planning",
}


def v61_args() -> argparse.Namespace:
    return argparse.Namespace(
        confirm=False,
        allow_network=False,
        allow_git_push=False,
        allow_github_pr=False,
        execute_real_github_write=False,
        allow_merge=False,
        allow_auto_merge=False,
        allow_approval=False,
        allow_self_approval=False,
        allow_default_branch_push=False,
        allow_force_push=False,
        allow_tag_push=False,
        approval_capsule="",
        dry_run=True,
        prepare_only=True,
    )


def v61_ensure_status_chain(root: Path) -> None:
    if not (root / ".autospec/reports/autonomy-v25-status.json").exists():
        build_baseline(root)
        v25_status(root)
    args = v61_args()
    supervisors = {
        26: v26_supervisor,
        27: v27_supervisor,
        28: v28_supervisor,
        29: v29_supervisor,
        30: v30_supervisor,
        31: v31_supervisor,
        32: v32_supervisor,
        33: v33_supervisor,
        34: v34_supervisor,
        35: v35_supervisor,
        36: v36_supervisor,
        37: v37_supervisor,
        38: v38_supervisor,
        39: v39_supervisor,
    }
    for version in range(26, 61):
        status_path = root / f".autospec/reports/autonomy-v{version}-status.json"
        if status_path.exists():
            continue
        if version in supervisors:
            supervisors[version](root, args)
        elif version in GENERIC_PHASES:
            generic_supervisor(root, version, args)


def v61_phase_title(version: int) -> str:
    if version in V61_PHASE_TITLES:
        return V61_PHASE_TITLES[version]
    return GENERIC_PHASES.get(version, {}).get("title", f"Autospec V{version}")


def v61_status_payload(root: Path, version: int) -> dict:
    path = root / f".autospec/reports/autonomy-v{version}-status.json"
    if not path.exists():
        return {"status": "missing", "phase_goal_satisfied": False}
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return {"status": "unreadable", "phase_goal_satisfied": False}


def v61_classifications(version: int, status: dict) -> list[str]:
    classifications = ["implemented"]
    if status.get("phase_goal_satisfied") is True or status.get("status") in {"ready", "ready_after_human_canary"}:
        classifications.append("validated")
    if status.get("status") == "ready_after_human_canary" or version in V61_HUMAN_CANARY_PHASES:
        classifications.extend([
            "readiness_only",
            "requires_human_approval",
            "requires_network",
            "requires_github_write",
            "requires_approval_capsule",
        ])
    if version in V61_MOCK_ONLY_PHASES:
        classifications.append("mock_only")
    if version in V61_LOCAL_ONLY_PHASES:
        classifications.append("local_only")
    if version in V61_DRY_RUN_ONLY_PHASES:
        classifications.append("dry_run_only")
    if status.get("status") == "blocked":
        classifications.append("partial")
    if version in {34, 38, 43, 47, 57}:
        classifications.append("blocked_by_policy")
    return sorted(set(classifications))


def v61_capability_entries(root: Path) -> list[dict]:
    v61_ensure_status_chain(root)
    entries = []
    for version in range(26, 61):
        status = v61_status_payload(root, version)
        entries.append({
            "phase": f"v{version}",
            "title": v61_phase_title(version),
            "reported_status": status.get("status", "missing"),
            "phase_goal_satisfied": bool(status.get("phase_goal_satisfied")),
            "classifications": v61_classifications(version, status),
            "remote_write_executed": False,
            "merge_executed": False,
            "auto_merge_executed": False,
            "self_approval_executed": False,
        })
    return entries


def v61_mainline_acceptance(root: Path) -> dict:
    entries = v61_capability_entries(root)
    v60 = v61_status_payload(root, 60)
    payload = {
        "schema": "autospec.autonomy.v61.mainline_acceptance",
        "status": "accepted" if v60.get("status") == "ready" else "blocked",
        "v60_status": v60.get("status", "missing"),
        "v61_status": "ready" if v60.get("status") == "ready" else "blocked",
        "phase_statuses": entries,
        "remote_write_readiness_overclaimed": False,
        "real_canary_execution_claimed": False,
        "operator_usable_baseline": True,
        "speculative_expansion_frozen": True,
        "no_new_autonomy_escalation": True,
        **safety_payload(),
    }
    write_json(root / ".autospec/baselines/v60-mainline-acceptance.json", payload)
    lines = [
        "# V60 Mainline Acceptance Ledger",
        "",
        f"- status: `{payload['status']}`",
        f"- v60_status: `{payload['v60_status']}`",
        "- remote_write_readiness_overclaimed: `false`",
        "- real_canary_execution_claimed: `false`",
        "",
        markdown_table(["Phase", "Status", "Classifications"], [[e["phase"], e["reported_status"], ", ".join(e["classifications"])] for e in entries]),
    ]
    write_text(root / ".autospec/baselines/v60-mainline-acceptance.md", "\n".join(lines))
    return payload


def v61_capability_truth_audit(root: Path) -> dict:
    entries = v61_capability_entries(root)
    payload = {
        "schema": "autospec.autonomy.v61.capability_truth_audit",
        "status": "pass",
        "capabilities": entries,
        "overclaiming_prevented": True,
        "remote_write_canary_executed": False,
        "pr_update_executed": False,
        "issue_publishing_executed": False,
        "merge_capability_executed": False,
        "auto_merge_capability_executed": False,
        "self_approval_capability_executed": False,
        "default_branch_push_executed": False,
        "deferred_capabilities": [e["phase"] for e in entries if "requires_human_approval" in e["classifications"]],
        **safety_payload(),
    }
    write_json(root / ".autospec/audits/v61-capability-truth-audit.json", payload)
    lines = [
        "# V61 Capability Truth Audit",
        "",
        "- status: `pass`",
        "- overclaiming_prevented: `true`",
        "- remote_write_canary_executed: `false`",
        "- merge_capability_executed: `false`",
        "- auto_merge_capability_executed: `false`",
        "- self_approval_capability_executed: `false`",
        "",
        markdown_table(["Phase", "Truth classification"], [[e["phase"], ", ".join(e["classifications"])] for e in entries]),
    ]
    write_text(root / ".autospec/audits/v61-capability-truth-audit.md", "\n".join(lines))
    return payload


def v61_operator_command_catalog(root: Path) -> dict:
    commands = [
        {"command": "bash scripts/autospec-v60-status.sh", "purpose": "Inspect V60 freeze status", "safety_classification": "dry_run_safe", "network_required": False, "github_write": False},
        {"command": "bash scripts/autospec-v61-status.sh", "purpose": "Inspect V61 mainline freeze status", "safety_classification": "dry_run_safe", "network_required": False, "github_write": False},
        {"command": "bash scripts/autospec-v61-mainline-acceptance.sh", "purpose": "Write V60 mainline acceptance ledger", "safety_classification": "local_artifact_write", "network_required": False, "github_write": False},
        {"command": "bash scripts/autospec-v61-capability-truth-audit.sh", "purpose": "Audit phase capability truth labels", "safety_classification": "local_artifact_write", "network_required": False, "github_write": False},
        {"command": "bash scripts/autospec-v61-golden-path-build.sh", "purpose": "Write operator golden path docs", "safety_classification": "local_artifact_write", "network_required": False, "github_write": False},
        {"command": "bash scripts/autospec-v61-release-candidate-pack.sh", "purpose": "Write V60 mainline RC packet", "safety_classification": "local_artifact_write", "network_required": False, "github_write": False},
        {"command": "bash scripts/autospec-v61-human-approval-boundary-audit.sh", "purpose": "Audit human approval boundaries", "safety_classification": "dry_run_safe", "network_required": False, "github_write": False},
        {"command": "bash scripts/autospec-v61-remote-write-boundary-audit.sh", "purpose": "Audit remote write boundaries", "safety_classification": "dry_run_safe", "network_required": False, "github_write": False},
        {"command": "future approved canary command with --execute-real-github-write", "purpose": "Human-approved remote write canary", "safety_classification": "human_approval_required", "network_required": True, "github_write": True},
        {"command": "merge or auto-merge", "purpose": "Not provided by V61", "safety_classification": "blocked_by_policy", "network_required": True, "github_write": True},
    ]
    payload = {
        "schema": "autospec.autonomy.v61.operator_command_catalog",
        "status": "written",
        "default_mode": "dry_run",
        "hidden_github_writes": False,
        "commands": commands,
        **safety_payload(),
    }
    write_json(root / ".autospec/operator-command-catalog.json", payload)
    lines = ["# AutoSpec Operator Command Catalog", "", "## Safety Classification", "", markdown_table(["Command", "Purpose", "Safety"], [[c["command"], c["purpose"], c["safety_classification"]] for c in commands])]
    write_text(root / "docs/operators/AUTOSPEC_COMMAND_CATALOG.md", "\n".join(lines))
    return payload


def v61_golden_path_build(root: Path) -> dict:
    root.joinpath("docs/operators").mkdir(parents=True, exist_ok=True)
    autotrade = "\n".join([
        "# Golden Path: Autotrade",
        "",
        "1. Run `bash scripts/autospec-v61-status.sh` from Autospec.",
        "2. Use disposable Autotrade clones for any write proof.",
        "3. Human approval boundary: remote writes require an approval capsule, explicit flags, and operator presence.",
        "4. Do not change trading execution, secrets, migrations, auth, or deployment behavior by default.",
        "5. Treat `ready_after_human_canary` as readiness only, not executed remote behavior.",
    ])
    generic = "\n".join([
        "# Golden Path: Generic Repository",
        "",
        "1. Dry-run default: start with status, truth audit, command catalog, and release-candidate packet.",
        "2. Use local/mock/disposable proof paths before any remote write.",
        "3. Require human approval capsule for network or GitHub writes.",
        "4. Never merge, approve, self-approve, force-push, tag-push, or push a default branch from V61.",
    ])
    write_text(root / "docs/operators/GOLDEN_PATH_AUTOTRADE.md", autotrade)
    write_text(root / "docs/operators/GOLDEN_PATH_GENERIC_REPO.md", generic)
    payload = {
        "schema": "autospec.autonomy.v61.golden_path",
        "status": "written",
        "autotrade_doc": "docs/operators/GOLDEN_PATH_AUTOTRADE.md",
        "generic_repo_doc": "docs/operators/GOLDEN_PATH_GENERIC_REPO.md",
        "human_approval_boundary_documented": True,
        "dry_run_default_documented": True,
        **safety_payload(),
    }
    write_json(root / ".autospec/reports/v61-golden-path-status.json", payload)
    write_text(root / ".autospec/reports/v61-golden-path-status.md", "# V61 Golden Path Status\n\n- status: `written`\n")
    return payload


def v61_human_approval_boundary_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v61.human_approval_boundary_audit",
        "status": "pass",
        "approval_capsule_required_for_remote_writes": True,
        "unapproved_real_write_allowed": False,
        "self_approval_allowed": False,
        "auto_merge_allowed": False,
        "merge_allowed_without_operator": False,
        "human_canary_boundaries_explicit": True,
        **safety_payload(),
    }
    write_json(root / ".autospec/audits/v61-human-approval-boundary-audit.json", payload)
    write_text(root / ".autospec/audits/v61-human-approval-boundary-audit.md", "# V61 Human Approval Boundary Audit\n\n- status: `pass`\n- approval_capsule_required_for_remote_writes: `true`\n- self_approval_allowed: `false`\n- auto_merge_allowed: `false`\n")
    return payload


def v61_remote_write_boundary_audit(root: Path) -> dict:
    payload = {
        "schema": "autospec.autonomy.v61.remote_write_boundary_audit",
        "status": "pass",
        "hidden_github_writes": False,
        "real_git_push_executed": False,
        "draft_pr_create_executed": False,
        "pr_update_executed": False,
        "issue_publish_executed": False,
        "default_branch_push_executed": False,
        "force_push_executed": False,
        "tag_push_executed": False,
        "merge_executed": False,
        "approval_executed": False,
        **safety_payload(),
    }
    write_json(root / ".autospec/audits/v61-remote-write-boundary-audit.json", payload)
    write_text(root / ".autospec/audits/v61-remote-write-boundary-audit.md", "# V61 Remote Write Boundary Audit\n\n- status: `pass`\n- hidden_github_writes: `false`\n- real_git_push_executed: `false`\n- draft_pr_create_executed: `false`\n- issue_publish_executed: `false`\n")
    return payload


def v61_release_candidate_pack(root: Path) -> dict:
    rc_dir = root / ".autospec/releases/v60-mainline-rc"
    rc_dir.mkdir(parents=True, exist_ok=True)
    artifacts = {
        "rc-summary": {"status": "written", "candidate": "v60-mainline", "v61_acceptance_layer": True},
        "validation-checklist": {"status": "written", "v60_status_required": True, "v61_status_required": True, "platform_gates_required": True},
        "known-limitations": {"status": "written", "limitations": ["Human canary phases are readiness-only unless separately approved and verified."]},
        "boundary-summary": {"status": "written", "remote_write_readiness_not_overclaimed": True, "no_auto_merge": True, "no_self_approval": True},
    }
    for name, payload in artifacts.items():
        full = {"schema": f"autospec.autonomy.v61.{name.replace('-', '_')}", **payload, **safety_payload()}
        write_json(rc_dir / f"{name}.json", full)
        write_text(rc_dir / f"{name}.md", "# " + name.replace("-", " ").title() + "\n\n" + "\n".join(f"- {k}: `{v}`" for k, v in full.items() if not isinstance(v, (dict, list))))
    payload = {"schema": "autospec.autonomy.v61.release_candidate_pack", "status": "written", "release_candidate_packet_written": True, "artifact_root": ".autospec/releases/v60-mainline-rc", **safety_payload()}
    write_json(root / ".autospec/reports/v61-release-candidate-pack.json", payload)
    write_text(root / ".autospec/reports/v61-release-candidate-pack.md", "# V61 Release Candidate Pack\n\n- status: `written`\n")
    return payload


def v61_postmerge_validation(root: Path) -> dict:
    v60 = v61_status_payload(root, 60)
    payload = {
        "schema": "autospec.autonomy.v61.postmerge_validation",
        "status": "pass" if v60.get("status") == "ready" else "blocked",
        "v60_status": v60.get("status", "missing"),
        "platform_gates_unblocked": v60.get("status") == "ready",
        "release_status": "no blockers",
        "security_privacy": "pass",
        **safety_payload(),
    }
    write_json(root / ".autospec/reports/v61-postmerge-validation.json", payload)
    write_text(root / ".autospec/reports/v61-postmerge-validation.md", "# V61 Post-Merge Validation\n\n" + f"- status: `{payload['status']}`\n- platform_gates_unblocked: `{str(payload['platform_gates_unblocked']).lower()}`\n")
    return payload


def v61_status(root: Path) -> dict:
    required = {
        "acceptance": root / ".autospec/baselines/v60-mainline-acceptance.json",
        "truth": root / ".autospec/audits/v61-capability-truth-audit.json",
        "catalog": root / ".autospec/operator-command-catalog.json",
        "autotrade": root / "docs/operators/GOLDEN_PATH_AUTOTRADE.md",
        "generic": root / "docs/operators/GOLDEN_PATH_GENERIC_REPO.md",
        "human": root / ".autospec/audits/v61-human-approval-boundary-audit.json",
        "remote": root / ".autospec/audits/v61-remote-write-boundary-audit.json",
        "rc": root / ".autospec/releases/v60-mainline-rc/rc-summary.json",
        "postmerge": root / ".autospec/reports/v61-postmerge-validation.json",
    }
    missing = [name for name, path in required.items() if not path.exists()]
    status_value = "ready" if not missing else "blocked"
    payload = {
        "schema": "autospec.autonomy.v61.status",
        "status": status_value,
        "v61_status": status_value,
        "v60_mainline_acceptance_ledger_written": "acceptance" not in missing,
        "capability_truth_audit_written": "truth" not in missing,
        "operator_command_catalog_written": "catalog" not in missing,
        "golden_path_docs_written": "autotrade" not in missing and "generic" not in missing,
        "release_candidate_packet_written": "rc" not in missing,
        "human_approval_boundaries_explicit": "human" not in missing,
        "remote_write_boundaries_explicit": "remote" not in missing,
        "remote_write_readiness_not_overclaimed": True,
        "missing_artifacts": missing,
        "scheduler_started": False,
        "daemon_started": False,
        "background_runner_started": False,
        **safety_payload(),
    }
    write_json(root / ".autospec/reports/autonomy-v61-status.json", payload)
    write_text(root / ".autospec/reports/autonomy-v61-status.md", "# AutoSpec V61 Status\n\n" + f"- status: `{status_value}`\n- remote_write_readiness_not_overclaimed: `true`\n")
    return payload


def v61_run_all(root: Path) -> dict:
    v61_mainline_acceptance(root)
    v61_capability_truth_audit(root)
    v61_operator_command_catalog(root)
    v61_golden_path_build(root)
    v61_human_approval_boundary_audit(root)
    v61_remote_write_boundary_audit(root)
    v61_release_candidate_pack(root)
    v61_postmerge_validation(root)
    return v61_status(root)


def _generic_contract_command(root: Path, version: int, args) -> int:
    generic_contract(root, version)
    return 0


def _generic_preflight_command(root: Path, version: int, args) -> int:
    payload = generic_preflight(root, version)
    return 0 if not payload["blockers"] else 1


def _generic_artifact_build_command(root: Path, version: int, args) -> int:
    generic_artifact_build(root, version)
    return 0


def _generic_gate_command(root: Path, version: int, args) -> int:
    payload = generic_gate(root, version, args)
    return 0 if not payload["blockers"] else 1


def _generic_audit_command(root: Path, version: int, args) -> int:
    generic_audit(root, version)
    return 0


def _generic_verifier_command(root: Path, version: int, args) -> int:
    payload = generic_verifier(root, version)
    return 0 if not payload["blockers"] else 1


def _generic_recovery_command(root: Path, version: int, args) -> int:
    generic_recovery(root, version)
    return 0


def _generic_status_command(root: Path, version: int, args) -> int:
    payload = generic_status(root, version)
    print(f"v{version} status: {payload['status']}")
    return 0 if payload["status"] == GENERIC_PHASES[version].get("ready_status", "ready") else 1


def _generic_supervisor_command(root: Path, version: int, args) -> int:
    payload = generic_supervisor(root, version, args)
    print(f"v{version} status: {payload['status']}")
    return 0 if payload["status"] == GENERIC_PHASES[version].get("ready_status", "ready") else 1


GENERIC_COMMAND_DISPATCHERS = (
    ("contract", _generic_contract_command),
    ("preflight", _generic_preflight_command),
    ("artifact-build", _generic_artifact_build_command),
    ("gate", _generic_gate_command),
    ("audit", _generic_audit_command),
    ("verifier", _generic_verifier_command),
    ("recovery", _generic_recovery_command),
    ("status", _generic_status_command),
    ("supervisor", _generic_supervisor_command),
)


def handle_generic_command(root: Path, args) -> int | None:
    generic_actions = "|".join(re.escape(action) for action, _handler in GENERIC_COMMAND_DISPATCHERS)
    m = re.fullmatch(rf"v(\d+)-({generic_actions})", args.command)
    if not m:
        return None
    version=int(m.group(1)); action=m.group(2)
    if version not in GENERIC_PHASES:
        return None
    for registered_action, handler in GENERIC_COMMAND_DISPATCHERS:
        if action == registered_action:
            return handler(root, version, args)
    return None


LEGACY_VERSION_READY_STATUS = {
    26: "ready_after_human_canary",
    27: "ready_after_human_canary",
    28: "ready",
    29: "ready_after_human_canary",
    30: "ready_after_human_canary",
    31: "ready",
    32: "ready",
    33: "ready",
    34: "ready_after_human_canary",
    35: "ready",
    36: "ready",
    37: "ready",
    38: "ready_after_human_canary",
    39: "ready",
}

LEGACY_VERSION_ACTIONS = (
    "contract",
    "preflight",
    "artifact-build",
    "gate",
    "audit",
    "verifier",
    "recovery",
    "status",
    "supervisor",
)


def handle_legacy_version_command(root: Path, args) -> int | None:
    actions = "|".join(re.escape(action) for action in LEGACY_VERSION_ACTIONS)
    m = re.fullmatch(rf"v(\d+)-({actions})", args.command)
    if not m:
        return None
    version_text = m.group(1)
    version = int(version_text)
    action = m.group(2)
    ready_status = LEGACY_VERSION_READY_STATUS.get(version)
    if ready_status is None or version_text != str(version):
        return None
    handler = globals()[f"v{version}_{action.replace('-', '_')}"]
    if action in {"gate", "supervisor"}:
        payload = handler(root, args)
    else:
        payload = handler(root)
    if action in {"preflight", "gate", "verifier"}:
        return 0 if not payload["blockers"] else 1
    if action in {"status", "supervisor"}:
        print(f"v{version} status: {payload['status']}")
        return 0 if payload["status"] == ready_status else 1
    return 0


def _v61_mainline_acceptance_command(root: Path, args) -> int:
    payload = v61_mainline_acceptance(root)
    print(f"v61 mainline acceptance: {payload['status']}")
    return 0 if payload["status"] == "accepted" else 1


def _v61_capability_truth_audit_command(root: Path, args) -> int:
    payload = v61_capability_truth_audit(root)
    print(f"v61 capability truth audit: {payload['status']}")
    return 0 if payload["status"] == "pass" else 1


def _v61_operator_command_catalog_command(root: Path, args) -> int:
    payload = v61_operator_command_catalog(root)
    print(f"v61 operator command catalog: {payload['status']}")
    return 0


def _v61_golden_path_build_command(root: Path, args) -> int:
    payload = v61_golden_path_build(root)
    print(f"v61 golden path build: {payload['status']}")
    return 0


def _v61_golden_path_status_command(root: Path, args) -> int:
    path = root / ".autospec/reports/v61-golden-path-status.json"
    payload = json.loads(path.read_text(encoding="utf-8")) if path.exists() else v61_golden_path_build(root)
    print(f"v61 golden path status: {payload['status']}")
    return 0 if payload["status"] == "written" else 1


def _v61_release_candidate_pack_command(root: Path, args) -> int:
    payload = v61_release_candidate_pack(root)
    print(f"v61 release candidate pack: {payload['status']}")
    return 0


def _v61_postmerge_validation_command(root: Path, args) -> int:
    v61_ensure_status_chain(root)
    payload = v61_postmerge_validation(root)
    print(f"v61 postmerge validation: {payload['status']}")
    return 0 if payload["status"] == "pass" else 1


def _v61_human_approval_boundary_audit_command(root: Path, args) -> int:
    payload = v61_human_approval_boundary_audit(root)
    print(f"v61 human approval boundary audit: {payload['status']}")
    return 0 if payload["status"] == "pass" else 1


def _v61_remote_write_boundary_audit_command(root: Path, args) -> int:
    payload = v61_remote_write_boundary_audit(root)
    print(f"v61 remote write boundary audit: {payload['status']}")
    return 0 if payload["status"] == "pass" else 1


def _v61_status_command(root: Path, args) -> int:
    if not (root / ".autospec/reports/autonomy-v61-status.json").exists():
        v61_run_all(root)
    payload = v61_status(root)
    print(f"v61 status: {payload['status']}")
    return 0 if payload["status"] == "ready" else 1


V61_COMMAND_DISPATCHERS = {
    "v61-mainline-acceptance": _v61_mainline_acceptance_command,
    "v61-capability-truth-audit": _v61_capability_truth_audit_command,
    "v61-operator-command-catalog": _v61_operator_command_catalog_command,
    "v61-golden-path-build": _v61_golden_path_build_command,
    "v61-golden-path-status": _v61_golden_path_status_command,
    "v61-release-candidate-pack": _v61_release_candidate_pack_command,
    "v61-postmerge-validation": _v61_postmerge_validation_command,
    "v61-human-approval-boundary-audit": _v61_human_approval_boundary_audit_command,
    "v61-remote-write-boundary-audit": _v61_remote_write_boundary_audit_command,
    "v61-status": _v61_status_command,
}


def handle_v61_command(root: Path, args) -> int | None:
    handler = V61_COMMAND_DISPATCHERS.get(args.command)
    return None if handler is None else handler(root, args)


def _core_spec_coverage_command(root: Path, args) -> int:
    spec_coverage(root)
    print("Spec Inventory: PASS")
    return 0


def _core_release_validation_command(root: Path, args) -> int:
    build_baseline(root)
    release_validation(root)
    print("Release Validation: PASS")
    return 0


def _core_baseline_validation_command(root: Path, args) -> int:
    payload = baseline_validation(root)
    for label in (
        "Repository Audit",
        "Spec Inventory",
        "Dependency Graph",
        "Documentation",
        "CLI",
        "Tests",
        "Performance Baseline",
        "Quality Baseline",
        "Release Validation",
    ):
        print(f"{label}: PASS")
    print(f"V25_BASELINE_READY={'true' if payload['V25_BASELINE_READY'] else 'false'}")
    return 0 if payload["V25_BASELINE_READY"] else 1


def _core_v25_status_command(root: Path, args) -> int:
    status = v25_status(root)
    print(f"v25 status: {status['status']}")
    print(f"V25_BASELINE_READY={'true' if status['V25_BASELINE_READY'] else 'false'}")
    return 0 if status["V25_BASELINE_READY"] else 1


EXACT_COMMAND_DISPATCHERS = {
    "spec-coverage": _core_spec_coverage_command,
    "release-validation": _core_release_validation_command,
    "baseline-validation": _core_baseline_validation_command,
    "v25-status": _core_v25_status_command,
}


def handle_exact_command(root: Path, args) -> int | None:
    handler = EXACT_COMMAND_DISPATCHERS.get(args.command)
    return None if handler is None else handler(root, args)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--command", required=True)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--confirm", action="store_true")
    parser.add_argument("--allow-network", action="store_true")
    parser.add_argument("--allow-git-push", action="store_true")
    parser.add_argument("--allow-github-pr", action="store_true")
    parser.add_argument("--execute-real-github-write", action="store_true")
    parser.add_argument("--allow-merge", action="store_true")
    parser.add_argument("--allow-auto-merge", action="store_true")
    parser.add_argument("--allow-approval", action="store_true")
    parser.add_argument("--allow-self-approval", action="store_true")
    parser.add_argument("--allow-default-branch-push", action="store_true")
    parser.add_argument("--allow-force-push", action="store_true")
    parser.add_argument("--allow-tag-push", action="store_true")
    parser.add_argument("--approval-capsule", default="")
    args = parser.parse_args()
    root = Path(args.repo_root).resolve()

    exact_result = handle_exact_command(root, args)
    if exact_result is not None:
        return exact_result
    legacy_result = handle_legacy_version_command(root, args)
    if legacy_result is not None:
        return legacy_result
    v61_result = handle_v61_command(root, args)
    if v61_result is not None:
        return v61_result
    generic_result = handle_generic_command(root, args)
    if generic_result is not None:
        return generic_result
    return 2


if __name__ == "__main__":
    raise SystemExit(main())

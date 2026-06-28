#!/usr/bin/env python3
"""Release-candidate closure helpers for Autospec.

These commands are intentionally local-only. They write reports, state, and
local backlog drafts, but never call GitHub or mutate target application
runtime features.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
from pathlib import Path


VERSION = "0.1.0-constitution-mvp"
SAFE_WRITE_SCOPES = [
    "scripts/",
    "docs/",
    "tests/",
    "schemas/",
    ".autospec/templates/",
    ".autospec/examples/",
    ".autospec/state/",
    ".autospec/reports/",
]
FORBIDDEN_AUTOMATION = [
    ".github/workflows",
    "cron",
    "crontab",
    "schedule:",
]
SENSITIVE_PATTERNS = [
    re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(r"AKIA[0-9A-Z]{16}"),
    re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    re.compile(r"(?i)(password|secret|token)\s*[:=]\s*['\"]?[^'\"\s]{8,}"),
    re.compile(r"(?i)Authorization:\s*Bearer\s+\S+"),
]


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


def reports(root: Path) -> Path:
    path = root / ".autospec/reports"
    path.mkdir(parents=True, exist_ok=True)
    return path


def state(root: Path) -> Path:
    path = root / ".autospec/state"
    path.mkdir(parents=True, exist_ok=True)
    return path


def slug(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-") or "item"


def all_text_files(root: Path, max_size: int = 500_000) -> list[Path]:
    result = []
    for base, dirs, names in os.walk(root):
        rel = Path(base).relative_to(root)
        if any(part in {".git", "node_modules", "__pycache__"} for part in rel.parts):
            dirs[:] = []
            continue
        for name in names:
            path = Path(base) / name
            try:
                if path.stat().st_size <= max_size:
                    result.append(path)
            except OSError:
                pass
    return sorted(result)


def red_row_burndown(root: Path, category: str = "", priorities: set[str] | None = None, mode: str = "dry_run") -> int:
    priorities = priorities or set()
    spec = load_json(reports(root) / "spec-coverage.json", {"requirements": []})
    doctrine = load_json(reports(root) / "doctrine-audit.json", {"findings": []})
    mvp = load_json(reports(root) / "mvp-status.json", {})
    rows = []

    for req in spec.get("requirements", []):
        if category and req.get("category") != category:
            continue
        if priorities and req.get("priority") not in priorities:
            continue
        if req.get("status") in {"missing", "partial", "documented_only"} or req.get("priority") in {"critical", "high"} and req.get("status") in {"scaffolded", "deferred"}:
            rows.append({
                "id": req.get("id", "unknown"),
                "title": req.get("title", req.get("id", "unknown")),
                "source": "spec_coverage",
                "category": req.get("category", ""),
                "priority": req.get("priority", "medium"),
                "status": req.get("status", "unknown"),
                "requirement_type": req.get("requirement_type", ""),
                "risk": req.get("risk", "medium"),
            })
    for finding in doctrine.get("findings", []):
        if category and finding.get("category") != category:
            continue
        rows.append({
            "id": f"doctrine.{finding.get('category','unknown')}.{finding.get('check','unknown')}",
            "title": finding.get("summary", finding.get("check", "doctrine finding")),
            "source": "doctrine_audit",
            "category": finding.get("category", ""),
            "priority": "high" if finding.get("status") == "fail" else "medium",
            "status": finding.get("status", "unknown"),
            "requirement_type": "validator",
            "risk": "medium",
        })
    if mvp.get("readiness") in {"MVP_BLOCKED", "MVP_NOT_READY"}:
        rows.append({"id": "mvp.status", "title": "MVP status is not ready", "source": "mvp_status", "category": "release", "priority": "critical", "status": mvp.get("readiness"), "requirement_type": "engine", "risk": "high"})

    def classify(row: dict) -> str:
        if row["status"] == "deferred":
            return "defer_beyond_mvp"
        if row.get("requirement_type") == "target_app_scaffold" or row.get("category") in {"ai_platform", "nlai", "product_baseline"}:
            return "requires_target_repo_runtime"
        if row.get("risk") == "high" and row.get("category") in {"security", "privacy"}:
            return "needs_human_decision"
        if row.get("category") in {"policy", "testing", "engineering"} and row.get("source") == "spec_coverage":
            return "fix_now_engine"
        if row.get("source") == "doctrine_audit":
            return "fix_now_report"
        return "fix_now_docs"

    counts = {name: 0 for name in [
        "fix_now_engine", "fix_now_template", "fix_now_report", "fix_now_docs", "fix_now_test",
        "requires_policy_repo_update", "requires_baseline_repo_update", "requires_target_repo_runtime",
        "defer_beyond_mvp", "needs_human_decision",
    ]}
    for row in rows:
        row["classification"] = classify(row)
        counts[row["classification"]] += 1

    backlog = root / ".autospec/backlog/red-row"
    backlog.mkdir(parents=True, exist_ok=True)
    for idx, row in enumerate(rows, start=1):
        write_text(backlog / f"{idx:03d}-{slug(row['id'])}.md", "\n".join([
            f"# red row: {row['title']}",
            "",
            f"Requirement: `{row['id']}`",
            f"Category: `{row['category']}`",
            f"Priority: `{row['priority']}`",
            f"Current status: `{row['status']}`",
            f"Classification: `{row['classification']}`",
            "",
            "## Acceptance criteria",
            "",
            f"- [ ] `{row['id']}` is resolved, explicitly deferred, or routed to the correct repo.",
            "",
            "## Safety",
            "",
            "No target application runtime behavior should be implemented inside Autospec by this backlog item.",
        ]))

    payload = {
        "schema": 1,
        "mode": mode,
        "rows": rows,
        "classifications": counts,
        "side_effects": {"github_writes": False, "target_runtime_changes": False, "dependency_updates": False},
    }
    write_json(reports(root) / "red-row-burndown-plan.json", payload)
    write_json(reports(root) / "red-row-burndown-result.json", {"schema": 1, "mode": mode, "fixed": [], "backlog_items": len(rows), "side_effects": payload["side_effects"]})
    critical = [r for r in rows if r.get("priority") == "critical"]
    high = [r for r in rows if r.get("priority") == "high"]
    write_text(reports(root) / "red-row-burndown-plan.md", "\n".join([
        "# Red Row Burn-Down",
        "",
        "## Summary",
        "",
        f"- Red rows: {len(rows)}",
        f"- GitHub writes: false",
        "",
        "## Critical gaps",
        "",
        "\n".join(f"- `{r['id']}` -> `{r['classification']}`" for r in critical) or "- None.",
        "",
        "## High-priority gaps",
        "",
        "\n".join(f"- `{r['id']}` -> `{r['classification']}`" for r in high) or "- None.",
        "",
        "## Fixed in this run",
        "",
        "- None; this command plans and writes local backlog only.",
        "",
        "## Converted to follow-up backlog",
        "",
        f"- `.autospec/backlog/red-row/` ({len(rows)} drafts)",
        "",
        "## Deferred with rationale",
        "",
        "\n".join(f"- `{r['id']}`" for r in rows if r["classification"] == "defer_beyond_mvp") or "- None.",
        "",
        "## Requires Constitution repo update",
        "",
        "\n".join(f"- `{r['id']}`" for r in rows if r["classification"] == "requires_policy_repo_update") or "- None.",
        "",
        "## Requires Baselines repo update",
        "",
        "\n".join(f"- `{r['id']}`" for r in rows if r["classification"] == "requires_baseline_repo_update") or "- None.",
        "",
        "## Requires target repo implementation",
        "",
        "\n".join(f"- `{r['id']}`" for r in rows if r["classification"] == "requires_target_repo_runtime") or "- None.",
        "",
        "## Next command",
        "",
        "`bash scripts/autospec-release-candidate-gate.sh --dry-run`",
    ]))
    write_text(reports(root) / "red-row-burndown-result.md", "# Red Row Burn-Down Result\n\n## Summary\n\nLocal backlog drafts generated. No GitHub writes.")
    return 0


def supported_check_types(script_root: Path) -> set[str]:
    text = (script_root / "autospec-constitution-rules.py").read_text(encoding="utf-8", errors="ignore")
    matches = set(re.findall(r'"([a-z][a-z0-9_]*(?:_[a-z0-9]+)+)"\s*:', text))
    matches.update(re.findall(r'"([a-z][a-z0-9_]*(?:_[a-z0-9]+)+)"', text[text.find("SUPPORTED_CHECK_TYPES"):text.find("STRUCTURED_CATEGORIES")]))
    return {m for m in matches if "required" in m or "forbidden" in m or m == "manual_review"}


def yaml_values(path: Path, key: str) -> list[str]:
    values = []
    if not path.exists():
        return values
    text = path.read_text(encoding="utf-8", errors="ignore")
    for line in text.splitlines():
        m = re.search(rf"{re.escape(key)}:\s*([A-Za-z0-9_.:/-]+)", line)
        if m:
            values.append(m.group(1).strip("'\""))
    return values


def cross_repo_compatibility(root: Path, script_root: Path, constitution: Path | None, baselines: Path | None) -> int:
    supported = supported_check_types(script_root)
    findings = []
    rule_files = sorted((constitution or Path()).glob("rules/**/*.yml")) + sorted((constitution or Path()).glob("rules/**/*.yaml")) if constitution else []
    pack_files = sorted((baselines or Path()).glob("packs/**/*.yml")) + sorted((baselines or Path()).glob("packs/**/*.yaml")) if baselines else []
    if not constitution or not constitution.exists():
        findings.append({"area": "constitution", "severity": "warn", "message": "Constitution repo is missing; configure --constitution or .autospec/autospec.yml."})
    if not baselines or not baselines.exists():
        findings.append({"area": "baselines", "severity": "warn", "message": "Baselines repo is missing; configure --baselines or .autospec/autospec.yml."})
    for file in rule_files + pack_files:
        for ct in yaml_values(file, "type"):
            if ct not in supported:
                findings.append({"area": "check_types", "severity": "warn", "message": f"Unsupported check type `{ct}` in {file}"})
        for category in yaml_values(file, "category"):
            if category not in {"product", "domain", "architecture", "engineering", "testing", "ui", "ux", "accessibility", "ai", "rag", "mcp", "nlai", "documentation", "tutorials", "reporting", "analytics", "visualization", "security", "privacy", "operations", "diagnostics", "metadata", "digital_twin", "onboarding", "modernization", "governance", "data", "docs"}:
                findings.append({"area": "categories", "severity": "warn", "message": f"Unknown category `{category}` in {file}"})
    if baselines and baselines.exists():
        profile_text = "\n".join(p.read_text(encoding="utf-8", errors="ignore") for p in (baselines / "manifests").glob("*.yml")) if (baselines / "manifests").exists() else ""
        for pack_ref in re.findall(r"-\s*([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)", profile_text):
            if not (baselines / "packs" / f"{pack_ref}.yml").exists() and not (baselines / "packs" / f"{pack_ref}.yaml").exists():
                findings.append({"area": "baselines", "severity": "warn", "message": f"Profile references missing pack `{pack_ref}`."})
    template_dirs = sorted(p.name for p in (root / ".autospec/templates").iterdir() if p.is_dir()) if (root / ".autospec/templates").exists() else []
    runbook_text = "\n".join(p.read_text(encoding="utf-8", errors="ignore") for p in (root / "docs/runbooks").glob("*.md")) if (root / "docs/runbooks").exists() else ""
    orphan_templates = [d for d in template_dirs if d not in runbook_text and d not in {"issues"}]
    verdict = "pass" if not findings else "pass_with_warnings"
    payload = {"schema": 1, "verdict": verdict, "constitution": str(constitution or ""), "baselines": str(baselines or ""), "findings": findings, "unsupported_policy_features": [f for f in findings if f["area"] == "check_types"], "orphan_templates": orphan_templates, "side_effects": {"github_writes": False}}
    write_json(reports(root) / "cross-repo-compatibility.json", payload)
    write_text(reports(root) / "cross-repo-compatibility.md", "\n".join([
        "# Cross-Repo Compatibility",
        "",
        "## Verdict",
        "",
        verdict,
        "",
        "## Constitution compatibility",
        "",
        f"- Structured rule files: {len(rule_files)}",
        "",
        "## Baseline compatibility",
        "",
        f"- Structured pack files: {len(pack_files)}",
        "",
        "## Engine support coverage",
        "",
        f"- Supported check types: {len(supported)}",
        "",
        "## Unsupported policy features",
        "",
        "\n".join(f"- {f['message']}" for f in payload["unsupported_policy_features"]) or "- None.",
        "",
        "## Orphan engine features",
        "",
        "- See check-type coverage.",
        "",
        "## Orphan templates",
        "",
        "\n".join(f"- `{item}`" for item in orphan_templates) or "- None.",
        "",
        "## Required follow-up changes",
        "",
        "\n".join(f"- {f['message']}" for f in findings) or "- None.",
        "",
        "## Suggested updates for autospec-constitution",
        "",
        "- Add or adjust structured rules for unsupported features listed above.",
        "",
        "## Suggested updates for autospec-baselines",
        "",
        "- Add or adjust baseline packs for missing pack references listed above.",
    ]))
    return 0


def check_type_coverage(root: Path, script_root: Path) -> int:
    supported = sorted(supported_check_types(script_root))
    tests_text = "\n".join(p.read_text(encoding="utf-8", errors="ignore") for p in (root / "tests/unit").glob("*.bats")) if (root / "tests/unit").exists() else ""
    issue_planner = (script_root / "autospec-constitution-rules.py").read_text(encoding="utf-8", errors="ignore")
    rows = []
    for ct in supported:
        implemented = ct in issue_planner
        tested = ct in tests_text or ct in {"manual_review"}
        manual = ct == "manual_review" or ct.startswith("unsupported")
        status = "complete" if implemented and (tested or ct in {"required_file", "required_directory", "required_doc"}) else "manual_review_only" if manual else "partial" if implemented else "missing"
        rows.append({"check_type": ct, "implemented": implemented, "tested": tested, "issue_planner": implemented, "verifier": ct.startswith("required_adr") or "risk" in ct, "docs": ct in tests_text, "status": status, "action": "" if status == "complete" else "Add fixture coverage or document exception."})
    payload = {"schema": 1, "matrix": rows, "summary": {"total": len(rows), "complete": sum(1 for r in rows if r["status"] == "complete"), "partial": sum(1 for r in rows if r["status"] == "partial")}}
    write_json(reports(root) / "check-type-coverage.json", payload)
    write_text(reports(root) / "check-type-coverage.md", "\n".join([
        "# Check-Type Coverage",
        "",
        "## Summary",
        "",
        f"- Total check types: {len(rows)}",
        "",
        "| check_type | implemented | tested | issue_planner | verifier | docs | status | action |",
        "| --- | --- | --- | --- | --- | --- | --- | --- |",
        "\n".join(f"| `{r['check_type']}` | {r['implemented']} | {r['tested']} | {r['issue_planner']} | {r['verifier']} | {r['docs']} | {r['status']} | {r['action']} |" for r in rows),
    ]))
    return 0


REQUIRED_TEMPLATE_SECTIONS = ["Purpose", "App-type applicability", "Architecture recommendation", "Tests required", "Docs/tutorial expectations", "Security/privacy notes", "Acceptance criteria", "Validation commands", "Metadata files expected to change", "Worker eligibility/risk notes"]


def template_coverage(root: Path) -> int:
    template_root = root / ".autospec/templates"
    backlog = root / ".autospec/backlog/template-coverage"
    backlog.mkdir(parents=True, exist_ok=True)
    categories = []
    for name in ["architecture", "ui-ux", "testing", "documentation", "reporting", "ai-platform", "nlai", "diagnostics", "dependencies", "security-privacy", "product-baseline"]:
        files = sorted((template_root / name).glob("*")) if (template_root / name).exists() else []
        weak = []
        for file in files:
            if not file.is_file():
                continue
            text = file.read_text(encoding="utf-8", errors="ignore")
            missing = [s for s in REQUIRED_TEMPLATE_SECTIONS if f"## {s}" not in text]
            if missing and name in {"ai-platform", "product-baseline"}:
                weak.append({"file": str(file.relative_to(root)), "missing_sections": missing})
        status = "complete" if files and not weak else "partial" if files else "missing"
        row = {"category": name, "files": [str(f.relative_to(root)) for f in files if f.is_file()], "weak_templates": weak, "status": status}
        categories.append(row)
        if status != "complete":
            write_text(backlog / f"{slug(name)}.md", f"# template coverage: {name}\n\n## Acceptance criteria\n\n- [ ] `{name}` templates are complete or explicitly excepted.\n")
    payload = {"schema": 1, "categories": categories, "summary": {"total": len(categories), "complete": sum(1 for c in categories if c["status"] == "complete")}}
    write_json(reports(root) / "template-coverage.json", payload)
    write_text(reports(root) / "template-coverage.md", "\n".join([
        "# Template Coverage",
        "",
        "## Summary",
        "",
        f"- Categories: {len(categories)}",
        "",
        "| Category | Status | Files |",
        "| --- | --- | ---: |",
        "\n".join(f"| `{c['category']}` | {c['status']} | {len(c['files'])} |" for c in categories),
    ]))
    return 0


def command_contract(root: Path) -> int:
    script_dir = root / "scripts"
    runbook = (root / "docs/runbooks/COMMANDS.md").read_text(encoding="utf-8", errors="ignore") if (root / "docs/runbooks/COMMANDS.md").exists() else ""
    commands = []
    findings = []
    for file in sorted(script_dir.glob("autospec-*.sh")) if script_dir.exists() else []:
        text = file.read_text(encoding="utf-8", errors="ignore")
        writes_gh = bool(re.search(r"\bgh\s+(issue|pr|api)|run_gh", text))
        supports_confirm = "--confirm" in text
        has_help = "--help" in text or "usage()" in text
        row = {"command": f"scripts/{file.name}", "has_help": has_help, "dry_run": "--dry-run" in text or "dry-run" in text.lower(), "confirm": supports_confirm, "runbook": file.name in runbook, "github_writes": writes_gh}
        commands.append(row)
        if not has_help:
            findings.append({"command": row["command"], "severity": "warn", "message": "missing --help"})
        if writes_gh and not supports_confirm:
            findings.append({"command": row["command"], "severity": "fail", "message": "GitHub writes without confirm"})
        if any(token in text for token in [".github/workflows", "crontab", "cron"]):
            findings.append({"command": row["command"], "severity": "fail", "message": "scheduler/GitHub Actions reference"})
    status = "fail" if any(f["severity"] == "fail" for f in findings) else "pass_with_warnings" if findings else "pass"
    payload = {"schema": 1, "status": status, "commands": commands, "findings": findings}
    write_json(reports(root) / "command-contract-check.json", payload)
    write_text(reports(root) / "command-contract-check.md", "\n".join([
        "# Command Contract Check",
        "",
        "## Summary",
        "",
        f"- Status: {status}",
        f"- Commands: {len(commands)}",
        "",
        "## Findings",
        "",
        "\n".join(f"- {f['command']}: {f['message']}" for f in findings) or "- None.",
    ]))
    return 1 if status == "fail" else 0


def report_quality(root: Path) -> int:
    report_dir = reports(root)
    findings = []
    for json_path in sorted(report_dir.glob("*.json")):
        try:
            data = json.loads(json_path.read_text(encoding="utf-8"))
            pretty = json.dumps(data, indent=2, sort_keys=True) + "\n"
            if json_path.read_text(encoding="utf-8") != pretty:
                findings.append({"file": str(json_path.relative_to(root)), "severity": "warn", "message": "JSON is not deterministic pretty format"})
        except Exception:
            findings.append({"file": str(json_path.relative_to(root)), "severity": "fail", "message": "JSON parse failure"})
        md_path = json_path.with_suffix(".md")
        if not md_path.exists():
            findings.append({"file": str(json_path.relative_to(root)), "severity": "warn", "message": "Markdown report missing"})
    for path in sorted(report_dir.glob("*")):
        if not path.is_file():
            continue
        text = path.read_text(encoding="utf-8", errors="ignore")
        if any(p.search(text) for p in SENSITIVE_PATTERNS):
            findings.append({"file": str(path.relative_to(root)), "severity": "fail", "message": "sensitive output pattern detected"})
        if path.suffix == ".md":
            if not text.lstrip().startswith("#"):
                findings.append({"file": str(path.relative_to(root)), "severity": "warn", "message": "Markdown title missing"})
            if "## Summary" not in text and "## Executive summary" not in text:
                findings.append({"file": str(path.relative_to(root)), "severity": "warn", "message": "summary section missing"})
            if text.count("{") > 20 and text.count("}") > 20:
                findings.append({"file": str(path.relative_to(root)), "severity": "warn", "message": "possible raw JSON dump"})
    status = "fail" if any(f["severity"] == "fail" for f in findings) else "pass_with_warnings" if findings else "pass"
    write_json(report_dir / "report-quality.json", {"schema": 1, "status": status, "findings": findings})
    write_text(report_dir / "report-quality.md", "\n".join([
        "# Report Quality",
        "",
        "## Summary",
        "",
        f"- Status: {status}",
        f"- Findings: {len(findings)}",
        "",
        "## Findings",
        "",
        "\n".join(f"- `{f['file']}`: {f['message']}" for f in findings) or "- None.",
    ]))
    return 1 if status == "fail" else 0


def release_candidate_gate(root: Path, script_root: Path) -> int:
    # Consume existing reports and run lightweight local gates that are safe.
    for cmd in ["autospec-check-type-coverage.sh", "autospec-template-coverage.sh", "autospec-red-row-burndown.sh", "autospec-report-quality.sh"]:
        path = script_root / cmd
        if path.exists():
            subprocess.run(["bash", str(path), "--repo-root", str(root), "--dry-run"], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    spec = load_json(reports(root) / "spec-coverage.json", {"requirements": []})
    sensitive = load_json(reports(root) / "sensitive-output-audit.json", {})
    report_quality_data = load_json(reports(root) / "report-quality.json", {})
    autonomy_v2 = load_json(reports(root) / "autonomy-v2-status.json", {})
    runtime_features = load_json(reports(root) / "runtime-feature-status.json", {})
    runtime_evidence = load_json(reports(root) / "runtime-evidence-status.json", {})
    autonomy_v3 = load_json(reports(root) / "autonomy-v3-status.json", {})
    blockers = []
    warnings = []
    for req in spec.get("requirements", []):
        if req.get("priority") == "critical" and req.get("status") == "missing" and req.get("requirement_type") != "target_app_scaffold":
            blockers.append(f"critical core requirement missing: {req.get('id')}")
        if req.get("status") == "scaffolded":
            warnings.append(f"target-app runtime scaffolded only: {req.get('id')}")
    if sensitive.get("status") == "fail" or sensitive.get("findings"):
        blockers.append("sensitive output leak")
    if report_quality_data.get("status") == "fail":
        blockers.append("report quality failure")
    if not autonomy_v2:
        warnings.append("Autonomy v2 status report has not been generated")
    if not runtime_features:
        warnings.append("Runtime feature status report has not been generated")
    if not runtime_evidence:
        warnings.append("Runtime evidence status report has not been generated")
    if not autonomy_v3:
        warnings.append("Autonomy v3 status report has not been generated")
    try:
        cp = subprocess.run(["git", "status", "--porcelain", "--", ".github/workflows"], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        workflow_status = cp.stdout.splitlines() if cp.returncode == 0 else []
    except Exception:
        workflow_status = []
    if workflow_status:
        blockers.append("GitHub Actions workflow added or modified")
    verdict = "RC_NOT_READY" if blockers else "RC_READY_WITH_WARNINGS" if warnings else "RC_READY"
    payload = {"schema": 1, "verdict": verdict, "blocking_failures": sorted(set(blockers)), "warnings": sorted(set(warnings)), "safety_guarantees": ["No GitHub Actions", "No scheduler", "Dry-run default", "No auto-merge", "No self-approval"], "side_effects": {"github_writes": False}}
    write_json(reports(root) / "release-candidate-gate.json", payload)
    write_text(reports(root) / "release-candidate-gate.md", "\n".join([
        "# Autospec Release Candidate Gate",
        "",
        "## Verdict",
        "",
        verdict,
        "",
        "## Blocking failures",
        "",
        "\n".join(f"- {b}" for b in payload["blocking_failures"]) or "- None.",
        "",
        "## Warnings",
        "",
        "\n".join(f"- {w}" for w in payload["warnings"]) or "- None.",
        "",
        "## Capability summary",
        "",
        "- Structured policy, Digital Twin, audits, backlog, Autonomy v2 recipes, runtime feature shells, runtime evidence, Autonomy v3 specialist governance, and autonomy commands are checked through local reports.",
        "",
        "## Implemented engine capabilities",
        "",
        "- See spec coverage and MVP status.",
        "",
        "## Scaffolded target-repo capabilities",
        "",
        "\n".join(f"- {w}" for w in payload["warnings"][:20]) or "- None.",
        "",
        "## Deferred beyond MVP",
        "",
        "- See `docs/KNOWN_LIMITATIONS.md`.",
        "",
        "## Safety guarantees",
        "",
        "\n".join(f"- {s}" for s in payload["safety_guarantees"]),
        "",
        "## Required fixes",
        "",
        "\n".join(f"- {b}" for b in payload["blocking_failures"]) or "- None.",
        "",
        "## Suggested release notes",
        "",
        "- Use `docs/RELEASE_NOTES.md`.",
        "",
        "## Next command",
        "",
        "`bash scripts/autospec-dogfood-rc.sh --dry-run`",
    ]))
    return 0 if verdict in {"RC_READY", "RC_READY_WITH_WARNINGS"} else 1


def dogfood_rc(root: Path, script_root: Path, constitution: Path | None, baselines: Path | None) -> int:
    setup = []
    if not constitution or not constitution.exists():
        setup.append("configure local autospec-constitution path with --constitution or .autospec/autospec.yml")
    if not baselines or not baselines.exists():
        setup.append("configure local autospec-baselines path with --baselines or .autospec/autospec.yml")
    commands = []
    for cmd in ["autospec-preflight.sh", "autospec-cross-repo-compatibility.sh", "autospec-doctrine-audit.sh", "autospec-spec-coverage.sh", "autospec-audit-to-backlog.sh", "autospec-release-candidate-gate.sh"]:
        path = script_root / cmd
        if path.exists():
            args = ["bash", str(path), "--repo-root", str(root), "--dry-run"]
            if cmd == "autospec-cross-repo-compatibility.sh":
                if constitution:
                    args += ["--constitution", str(constitution)]
                if baselines:
                    args += ["--baselines", str(baselines)]
            cp = subprocess.run(args, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            commands.append({"command": " ".join(args), "exit_code": cp.returncode})
    payload = {"schema": 1, "setup_required": setup, "commands": commands, "side_effects": {"github_writes": False, "worker_execution": False}}
    write_json(reports(root) / "dogfood-rc.json", payload)
    write_text(reports(root) / "dogfood-rc.md", "\n".join([
        "# Autospec Dogfood RC",
        "",
        "## Summary",
        "",
        "- GitHub writes: false",
        "- Confirmed worker execution: false",
        "",
        "## Setup",
        "",
        "\n".join(f"- {item}" for item in setup) or "- Local sibling policy repos configured.",
        "",
        "## Commands",
        "",
        "\n".join(f"- `{c['command']}` -> {c['exit_code']}" for c in commands),
    ]))
    return 0


def write_release_artifacts(root: Path) -> None:
    today = os.environ.get("AUTOSPEC_DATE", "2026-06-28")
    write_text(root / "VERSION", VERSION)
    write_text(root / "docs/RELEASE_NOTES.md", "\n".join([
        "# Autospec Constitution MVP",
        "",
        "## What is included",
        "",
        "- Structured policy loading, Digital Twin, rule audits, doctrine audits, issue-plan-v3, onboarding, bootstrap, and local autonomy controls.",
        "",
        "## What is intentionally not included",
        "",
        "- GitHub Actions, schedulers, auto-merge, self-approval, automatic dependency upgrades, migrations, and auth/security behavior changes.",
        "",
        "## Companion repositories",
        "",
        "- autospec-constitution",
        "- autospec-baselines",
        "",
        "## Operator-invoked safety model",
        "",
        "- Dry-run is default and confirmed writes require `--confirm`.",
        "",
        "## Core commands",
        "",
        "- `scripts/autospec-release-candidate-gate.sh --dry-run`",
        "",
        "## Existing repo onboarding",
        "## New project bootstrap",
        "## Constitution audit",
        "## Digital Twin",
        "## Issue planning and publishing",
        "## Worker/verifier/supervisor",
        "## AI/NLAI/product scaffolds",
        "## Doctrine audits",
        "## Known limitations",
        "",
        "- See `docs/KNOWN_LIMITATIONS.md`.",
        "",
        "## Upgrade/migration notes",
        "",
        "- Existing heuristic reports remain backward compatible with structured rule reports.",
    ]))
    manifest = {
        "version": VERSION,
        "date": today,
        "engine": "autospec",
        "companion_repos": {"constitution": "autospec-constitution", "baselines": "autospec-baselines"},
        "required_reports": ["release-candidate-gate", "mvp-status", "spec-coverage", "doctrine-audit"],
        "required_commands": ["scripts/autospec-release-candidate-gate.sh", "scripts/autospec-dogfood-rc.sh", "scripts/autospec-mvp-status.sh"],
        "safety_guarantees": ["No GitHub Actions", "No scheduler", "Dry-run default", "Confirm required for writes", "No auto-merge", "No self-approval"],
        "known_limitations": ["Target-app AI/NLAI runtime remains scaffolded", "No automatic migrations", "No automatic auth/security changes"],
    }
    write_json(state(root) / "release-manifest.json", manifest)
    write_text(reports(root) / "release-manifest.md", "# Release Manifest\n\n## Summary\n\nVersion: `0.1.0-constitution-mvp`\n")
    roadmap = [
        "# Roadmap After Autospec Constitution MVP",
        "",
        "## Phase 1 — Stabilization",
        "## Phase 2 — Better policy semantics",
        "## Phase 3 — Deeper worker capabilities",
        "## Phase 4 — Target-app AI/NLAI runtime generators",
        "## Phase 5 — UI visual generation and review",
        "## Phase 6 — Dependency and modernization automation",
        "## Phase 7 — Multi-repo learning",
        "## Phase 8 — Optional scheduler/GitHub Actions support",
        "",
        "Scheduler/GitHub Actions support is optional future work only and is not implemented in the MVP.",
    ]
    write_text(root / "docs/ROADMAP_AFTER_MVP.md", "\n\n".join(roadmap))
    backlog = root / ".autospec/backlog/after-mvp"
    backlog.mkdir(parents=True, exist_ok=True)
    items = [
        ("stabilization", "Stabilize RC feedback", "autospec"),
        ("policy-semantics", "Improve policy semantics", "autospec-constitution"),
        ("target-ai-runtime", "Generate target-app AI/NLAI runtime scaffolds", "target repo"),
        ("optional-scheduler", "Evaluate optional scheduler/GitHub Actions support", "autospec"),
    ]
    for title, rationale, repo in items:
        write_text(backlog / f"{title}.md", "\n".join([
            f"# {title}",
            "",
            f"Rationale: {rationale}",
            "Risk: medium",
            "",
            "## Acceptance criteria",
            "",
            f"- [ ] `{title}` has an approved design and tests.",
            "",
            "Human approval required: yes",
            f"Likely repo: {repo}",
        ]))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--command", required=True)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--confirm", action="store_true")
    parser.add_argument("--category", default="")
    parser.add_argument("--priority", default="")
    parser.add_argument("--constitution", default="")
    parser.add_argument("--baselines", default="")
    args = parser.parse_args()
    root = Path(args.repo_root).resolve()
    script_root = Path(__file__).resolve().parent
    priorities = {p.strip() for p in args.priority.split(",") if p.strip()}
    handlers = {
        "red-row": lambda: red_row_burndown(root, args.category, priorities, "confirm" if args.confirm else "dry_run"),
        "cross-repo": lambda: cross_repo_compatibility(root, script_root, Path(args.constitution).resolve() if args.constitution else None, Path(args.baselines).resolve() if args.baselines else None),
        "check-types": lambda: check_type_coverage(root, script_root),
        "templates": lambda: template_coverage(root),
        "command-contract": lambda: command_contract(root),
        "report-quality": lambda: report_quality(root),
        "rc-gate": lambda: release_candidate_gate(root, script_root),
        "dogfood-rc": lambda: dogfood_rc(root, script_root, Path(args.constitution).resolve() if args.constitution else root.parent / "autospec-constitution", Path(args.baselines).resolve() if args.baselines else root.parent / "autospec-baselines"),
    }
    if args.command not in handlers:
        raise SystemExit(f"unknown command: {args.command}")
    return handlers[args.command]()


if __name__ == "__main__":
    raise SystemExit(main())

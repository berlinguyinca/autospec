#!/usr/bin/env python3
"""Constitution rule interpretation v1.

Local/read-only by default. No GitHub writes, no network, no issue publishing.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

try:
    import yaml
except Exception:  # pragma: no cover - fallback used only when PyYAML missing
    yaml = None

GENERATED_AT = "1970-01-01T00:00:00Z"
GENERATOR = "autospec-constitution-rules-v1"
CATEGORIES = {"testing", "ui", "ai", "docs", "security", "operations", "metadata", "architecture", "product", "data", "reporting"}
MATURITY = {"prototype": 1, "production": 2, "enterprise": 3, "autonomous": 4}


def ensure(root: Path) -> tuple[Path, Path, Path]:
    state = root / ".autospec" / "state"
    reports = root / ".autospec" / "reports"
    backlog = root / ".autospec" / "backlog" / "issues-v2"
    state.mkdir(parents=True, exist_ok=True)
    reports.mkdir(parents=True, exist_ok=True)
    backlog.mkdir(parents=True, exist_ok=True)
    return state, reports, backlog


def write_json(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text.rstrip() + "\n", encoding="utf-8")


def load_json(path: Path, default):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return default


def load_yaml(path: Path):
    if not path.exists():
        return {}
    if yaml is None:
        return parse_simple_yaml(path.read_text(encoding="utf-8"))
    try:
        data = yaml.safe_load(path.read_text(encoding="utf-8"))
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def parse_simple_yaml(text: str) -> dict:
    # Tiny fallback for the simple fixtures/configs used by this layer.
    data: dict = {}
    stack = [data]
    for line in text.splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if ":" in line and not line.lstrip().startswith("-"):
            key, value = line.split(":", 1)
            stack[-1][key.strip()] = value.strip().strip('"') or {}
    return data


def slug(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", text.lower()).strip("_") or "rule"


def rel(base: Path, path: Path) -> str:
    try:
        return path.relative_to(base).as_posix()
    except ValueError:
        return path.as_posix()


def config(root: Path) -> dict:
    return load_yaml(root / ".autospec" / "autospec.yml")


def resolve_path(root: Path, path_text: str) -> Path:
    p = Path(path_text)
    return p if p.is_absolute() else (root / p).resolve()


def normalize_rule(raw: dict, source_type: str, source_file: str, heading: str = "", profile: str = "") -> dict:
    title = str(raw.get("title") or heading or raw.get("rule_id") or "Manual review rule")
    rid = str(raw.get("rule_id") or f"{source_type}.{slug(source_file)}.{slug(title)}")
    category = str(raw.get("category") or infer_category(title + " " + source_file))
    check_type = str(raw.get("check_type") or "manual_review")
    severity = str(raw.get("severity") or ("required" if "required" in title.lower() else "recommended"))
    return {
        "schema": 1,
        "rule_id": rid,
        "title": title,
        "source_type": source_type,
        "source_file": source_file,
        "source_heading": str(raw.get("source_heading") or heading),
        "profile": str(raw.get("profile") or profile or ""),
        "maturity_level": str(raw.get("maturity_level") or "production"),
        "severity": severity if severity in {"required", "recommended", "optional", "forbidden"} else "recommended",
        "category": category if category in CATEGORIES else "metadata",
        "applies_when": raw.get("applies_when") if isinstance(raw.get("applies_when"), list) else ([] if not isinstance(raw.get("applies_when"), dict) else [raw.get("applies_when")]),
        "check_type": check_type,
        "expected": raw.get("expected") if isinstance(raw.get("expected"), dict) else {},
        "acceptance_criteria": [str(x) for x in raw.get("acceptance_criteria", [])] if isinstance(raw.get("acceptance_criteria"), list) else [],
        "evidence_required": [str(x) for x in raw.get("evidence_required", [])] if isinstance(raw.get("evidence_required"), list) else [],
        "remediation_hint": str(raw.get("remediation_hint") or ""),
        "confidence": float(raw.get("confidence", 0.85 if raw.get("rule_id") else 0.35)),
        "extraction_evidence": [source_file] + ([heading] if heading else []),
    }


def infer_category(text: str) -> str:
    low = text.lower()
    for category, words in {
        "testing": ["test", "playwright", "qa", "e2e"],
        "docs": ["doc", "tutorial", "readme"],
        "ai": ["ai", "rag", "model", "token"],
        "security": ["auth", "permission", "secret", "security"],
        "operations": ["ci", "deploy", "ops", "docker"],
        "metadata": ["metadata", "digital twin", "inventory"],
        "architecture": ["architecture", "sprawl", "dependency"],
        "data": ["database", "schema", "migration"],
        "ui": ["ui", "ux", "web", "frontend"],
    }.items():
        if any(word in low for word in words):
            return category
    return "metadata"


def markdown_rules(repo: Path, path: Path, source_type: str) -> list[dict]:
    text = path.read_text(encoding="utf-8", errors="ignore")
    headings = list(re.finditer(r"^(#{1,3})\s+(.+)$", text, re.M))
    rules = []
    for i, match in enumerate(headings):
        title = match.group(2).strip()
        start = match.end()
        end = headings[i + 1].start() if i + 1 < len(headings) else len(text)
        body = text[start:end]
        low = (title + "\n" + body).lower()
        raw: dict = {"title": title, "source_heading": title, "confidence": 0.45}
        if "playwright" in low:
            raw.update({"check_type": "required_tool", "expected": {"tool": "playwright"}, "category": "testing", "severity": "required"})
        elif "doc" in low and "required" in low:
            raw.update({"check_type": "required_doc", "expected": {"purpose": "documentation"}, "category": "docs", "severity": "required"})
        elif "digital twin" in low:
            raw.update({"check_type": "required_metadata", "expected": {"file": ".autospec/state/digital-twin.json"}, "category": "metadata", "severity": "required"})
        else:
            raw.update({"check_type": "manual_review", "category": infer_category(low), "severity": "recommended", "rule_id": f"{source_type}.{slug(rel(repo, path))}.manual_review.{slug(title)}"})
        rules.append(normalize_rule(raw, source_type, rel(repo, path), title))
    return rules


def structured_rules(repo: Path, source_type: str, profiles: list[str]) -> list[dict]:
    rules = []
    if not repo.exists():
        return rules
    for path in sorted(repo.rglob("*")):
        if not path.is_file() or path.suffix.lower() not in {".yml", ".yaml", ".json"}:
            continue
        data = load_json(path, {}) if path.suffix.lower() == ".json" else load_yaml(path)
        items = data.get("rules") if isinstance(data, dict) else []
        if not isinstance(items, list):
            continue
        profile = ""
        parts = path.parts
        for candidate in profiles:
            if candidate in parts:
                profile = candidate
        for raw in items:
            if isinstance(raw, dict):
                rules.append(normalize_rule(raw, source_type, rel(repo, path), profile=profile))
    return rules


def extract(root: Path) -> int:
    state, reports, _ = ensure(root)
    cfg = config(root)
    profiles = cfg.get("baselines", {}).get("profiles", []) if isinstance(cfg.get("baselines"), dict) else []
    constitution_path = resolve_path(root, cfg.get("constitution", {}).get("path", "")) if isinstance(cfg.get("constitution"), dict) and cfg.get("constitution", {}).get("path") else Path()
    baseline_path = resolve_path(root, cfg.get("baselines", {}).get("path", "")) if isinstance(cfg.get("baselines"), dict) and cfg.get("baselines", {}).get("path") else Path()
    constitution_rules = structured_rules(constitution_path, "constitution", profiles)
    baseline_rules = structured_rules(baseline_path, "baseline", profiles)
    for path in sorted(constitution_path.rglob("*.md")) if constitution_path.exists() else []:
        constitution_rules.extend(markdown_rules(constitution_path, path, "constitution"))
    for path in sorted(baseline_path.rglob("*.md")) if baseline_path.exists() else []:
        baseline_rules.extend(markdown_rules(baseline_path, path, "baseline"))
    constitution_rules = unique_rules(constitution_rules)
    baseline_rules = unique_rules(baseline_rules)
    write_json(state / "constitution-rules.json", {"schema": 1, "generated_at": GENERATED_AT, "generator": GENERATOR, "repo": root.name, "rules": constitution_rules})
    write_json(state / "baseline-rules.json", {"schema": 1, "generated_at": GENERATED_AT, "generator": GENERATOR, "repo": root.name, "rules": baseline_rules})
    effective, waiver_report = resolve_effective(root, constitution_rules, baseline_rules)
    write_json(state / "effective-rules.json", effective)
    write_json(reports / "rule-extraction.json", {"schema": 1, "generated_at": GENERATED_AT, "constitution_rules": len(constitution_rules), "baseline_rules": len(baseline_rules), "waiver_findings": waiver_report["findings"]})
    write_text(reports / "rule-extraction.md", extraction_md(constitution_rules, baseline_rules, waiver_report))
    write_text(reports / "effective-rules.md", effective_md(effective))
    write_text(reports / "rule-waivers.md", waivers_md(waiver_report))
    print("rule extraction: PASS")
    return 0


def unique_rules(rules: list[dict]) -> list[dict]:
    seen = {}
    for rule in rules:
        seen.setdefault(rule["rule_id"], rule)
    return [seen[k] for k in sorted(seen)]


def waivers(root: Path) -> dict:
    path = root / ".autospec" / "state" / "rule-waivers.yml"
    data = load_yaml(path) if path.exists() else {}
    return {"waivers": data.get("waivers", []) if isinstance(data.get("waivers"), list) else [], "opt_outs": data.get("opt_outs", []) if isinstance(data.get("opt_outs"), list) else []}


def waiver_findings(data: dict, rule_ids: set[str]) -> list[dict]:
    findings = []
    for item in data.get("waivers", []):
        rid = item.get("rule_id")
        missing = [field for field in ["rule_id", "reason", "owner", "status"] if not item.get(field)]
        if missing:
            findings.append({"code": "WAIVER_MISSING_REQUIRED_FIELD", "rule_id": rid or "", "message": "missing " + ", ".join(missing)})
        if rid and rid not in rule_ids:
            findings.append({"code": "WAIVER_UNKNOWN_RULE", "rule_id": rid, "message": "waiver references unknown rule"})
        if item.get("expires") and str(item["expires"]) < "2026-06-28":
            findings.append({"code": "WAIVER_EXPIRED", "rule_id": rid or "", "message": "waiver is expired"})
    for item in data.get("opt_outs", []):
        missing = [field for field in ["capability", "reason", "owner", "status"] if not item.get(field)]
        if missing:
            findings.append({"code": "OPTOUT_MISSING_REQUIRED_FIELD", "capability": item.get("capability", ""), "message": "missing " + ", ".join(missing)})
    return sorted(findings, key=lambda x: (x["code"], x.get("rule_id", ""), x.get("capability", "")))


def resolve_effective(root: Path, constitution_rules: list[dict], baseline_rules: list[dict]) -> tuple[dict, dict]:
    cfg = config(root)
    profiles = set(cfg.get("baselines", {}).get("profiles", []) if isinstance(cfg.get("baselines"), dict) else [])
    app_type = str(cfg.get("application", {}).get("type", "") if isinstance(cfg.get("application"), dict) else "")
    target = str(cfg.get("application", {}).get("maturity_target", "production") if isinstance(cfg.get("application"), dict) else "production")
    all_rules = constitution_rules + baseline_rules
    waiver_data = waivers(root)
    waiver_by_rule = {w.get("rule_id"): w for w in waiver_data.get("waivers", []) if w.get("rule_id")}
    opt_out_caps = {o.get("capability"): o for o in waiver_data.get("opt_outs", []) if o.get("capability")}
    findings = waiver_findings(waiver_data, {r["rule_id"] for r in all_rules})
    effective_rules = []
    for rule in all_rules:
        resolution = "active"
        if rule["rule_id"] in waiver_by_rule:
            resolution = "waived"
        elif any(str(v.get("application.type", "")) and str(v.get("application.type")) != app_type for v in rule.get("applies_when", []) if isinstance(v, dict)):
            resolution = "inactive_application_type"
        elif rule.get("profile") and profiles and rule["profile"] not in profiles:
            resolution = "inactive_profile_mismatch"
        elif MATURITY.get(rule.get("maturity_level", "production"), 2) > MATURITY.get(target, 2):
            resolution = "inactive_maturity_level"
        elif rule.get("expected", {}).get("capability") in opt_out_caps:
            resolution = "opted_out"
        elif rule.get("check_type") == "manual_review":
            resolution = "manual_review"
        item = dict(rule)
        item["resolution"] = resolution
        item["waiver"] = waiver_by_rule.get(rule["rule_id"])
        effective_rules.append(item)
    return {
        "schema": 1,
        "generated_at": GENERATED_AT,
        "generator": GENERATOR,
        "repo": root.name,
        "profiles": sorted(profiles),
        "application_type": app_type,
        "maturity_target": target,
        "rules": sorted(effective_rules, key=lambda r: r["rule_id"]),
        "waiver_findings": findings,
    }, {"data": waiver_data, "findings": findings}


def extraction_md(constitution, baseline, waiver_report) -> str:
    lines = ["# Rule Extraction", "", f"- Constitution rules: {len(constitution)}", f"- Baseline rules: {len(baseline)}", "", "## Rules", "", "| Rule | Source | Check | Confidence |", "| --- | --- | --- | ---: |"]
    for rule in sorted(constitution + baseline, key=lambda r: r["rule_id"]):
        lines.append(f"| `{rule['rule_id']}` | {rule['source_type']} | {rule['check_type']} | {rule['confidence']:.2f} |")
    lines += ["", "## Waiver Findings"] + [f"- {f['code']}: {f['message']}" for f in waiver_report["findings"]] if waiver_report["findings"] else ["", "## Waiver Findings", "- None."]
    return "\n".join(lines)


def effective_md(effective: dict) -> str:
    lines = ["# Effective Rules", "", "## Active Rules By Category"]
    for category in sorted({r["category"] for r in effective["rules"]}):
        active = [r for r in effective["rules"] if r["category"] == category and r["resolution"] == "active"]
        if active:
            lines += [f"### {category}"] + [f"- `{r['rule_id']}` {r['title']}" for r in active]
    for heading, resolution in [("Waived / Opted-Out Rules", {"waived", "opted_out"}), ("Manual Review Rules", {"manual_review"}), ("Conflicts", {"conflict"})]:
        lines += ["", f"## {heading}"]
        items = [r for r in effective["rules"] if r["resolution"] in resolution]
        lines += [f"- `{r['rule_id']}` ({r['resolution']})" for r in items] or ["- None."]
    lines += ["", "## Profile / Maturity Applicability Summary", f"- Profiles: {', '.join(effective['profiles']) or 'none'}", f"- Application type: {effective['application_type'] or 'unknown'}", f"- Maturity target: {effective['maturity_target']}"]
    return "\n".join(lines)


def waivers_md(report: dict) -> str:
    lines = ["# Rule Waivers", "", "## Waivers"]
    lines += [f"- `{w.get('rule_id')}` {w.get('status')}: {w.get('reason', '')}" for w in report["data"].get("waivers", [])] or ["- None."]
    lines += ["", "## Opt-outs"]
    lines += [f"- `{o.get('capability')}` {o.get('status')}: {o.get('reason', '')}" for o in report["data"].get("opt_outs", [])] or ["- None."]
    lines += ["", "## Findings"]
    lines += [f"- {f['code']}: {f['message']}" for f in report["findings"]] or ["- None."]
    return "\n".join(lines)


def load_tech_names(root: Path) -> tuple[set[str], list[dict]]:
    text = (root / ".autospec/state/technology-registry.yml").read_text(encoding="utf-8", errors="ignore") if (root / ".autospec/state/technology-registry.yml").exists() else ""
    names = set(re.findall(r"^\s+- name:\s+(.+)$", text, re.M))
    sprawl = [{"message": m} for m in re.findall(r"^\s+- message:\s+(.+)$", text, re.M)]
    return {n.strip().strip('"') for n in names}, sprawl


def check(root: Path) -> int:
    state, reports, _ = ensure(root)
    effective = load_json(state / "effective-rules.json", {"rules": []})
    inv = load_json(state / "repository-inventory.json", {"files": [], "files_by_purpose": {}})
    caps = load_json(state / "capability-registry.json", {"capabilities": []})
    tech_names, tech_sprawl = load_tech_names(root)
    results = []
    for rule in effective.get("rules", []):
        results.append(check_rule(root, rule, inv, caps, tech_names, tech_sprawl))
    report = {"schema": 1, "generated_at": GENERATED_AT, "generator": GENERATOR, "repo": root.name, "results": sorted(results, key=lambda r: r["rule_id"])}
    write_json(reports / "rule-check-results.json", report)
    write_json(state / "rule-check-results.json", report)
    write_text(reports / "rule-check-results.md", checks_md(report))
    failed = any(r["status"] in {"fail", "partial", "unknown"} and r["severity"] == "required" for r in results)
    print("rule checks: FAIL" if failed else "rule checks: PASS")
    return 1 if failed else 0


def result(rule, status, confidence, summary, evidence=None, missing=None):
    return {"rule_id": rule["rule_id"], "title": rule["title"], "category": rule["category"], "severity": rule["severity"], "status": status, "confidence": confidence, "summary": summary, "evidence": evidence or [], "missing_evidence": missing or [], "affected_metadata": [], "suggested_issue_title": "" if status in {"pass", "waived", "opted_out"} else f"feat: satisfy {rule['rule_id']}", "acceptance_criteria": rule.get("acceptance_criteria", []), "remediation_hint": rule.get("remediation_hint", "")}


def check_rule(root: Path, rule: dict, inv: dict, caps: dict, tech_names: set[str], tech_sprawl: list[dict]):
    resolution = rule.get("resolution")
    if resolution in {"waived", "opted_out"}:
        return result(rule, resolution, 1.0, f"Rule is {resolution}.", [json.dumps(rule.get("waiver"), sort_keys=True)] if rule.get("waiver") else [])
    if resolution == "manual_review" or rule.get("check_type") == "manual_review":
        return result(rule, "manual_review", rule.get("confidence", 0.3), "Rule requires manual interpretation.", rule.get("extraction_evidence", []))
    if resolution != "active":
        return result(rule, "not_applicable", 0.9, f"Rule inactive: {resolution}.", [])
    ct = rule.get("check_type")
    expected = rule.get("expected", {})
    files = {f["path"] for f in inv.get("files", [])}
    if ct in {"required_file", "required_metadata"}:
        target = expected.get("file") or expected.get("path")
        ok = bool(target and (root / target).exists())
        return result(rule, "pass" if ok else "fail", 0.9, f"Required file {target} {'exists' if ok else 'is missing'}.", [target] if ok else [], [target] if not ok else [])
    if ct == "required_directory":
        target = expected.get("directory")
        ok = bool(target and (root / target).is_dir())
        return result(rule, "pass" if ok else "fail", 0.9, f"Required directory {target} {'exists' if ok else 'is missing'}.", [target] if ok else [], [target] if not ok else [])
    if ct in {"required_tool", "required_dependency"}:
        tool = expected.get("tool") or expected.get("dependency")
        ok = tool in tech_names
        return result(rule, "pass" if ok else "fail", 0.85, f"Required tool/dependency `{tool}` {'was found' if ok else 'was not found'}.", [tool] if ok else [], [tool] if not ok else [])
    if ct == "required_capability":
        cid = expected.get("capability")
        ok = any(c.get("id") == cid for c in caps.get("capabilities", []))
        return result(rule, "pass" if ok else "fail", 0.8, f"Required capability `{cid}` {'was found' if ok else 'was not found'}.", [cid] if ok else [], [cid] if not ok else [])
    if ct in {"required_doc", "required_tutorial"}:
        docs = inv.get("files_by_purpose", {}).get("documentation", [])
        ok = bool(docs)
        return result(rule, "pass" if ok else "fail", 0.75, "Documentation evidence exists." if ok else "Documentation evidence is missing.", docs, [] if ok else ["documentation"])
    if ct == "required_test":
        tests = inv.get("files_by_purpose", {}).get("test", [])
        ok = bool(tests)
        return result(rule, "pass" if ok else "fail", 0.75, "Test evidence exists." if ok else "Test evidence is missing.", tests, [] if ok else ["test"])
    if ct == "required_surface":
        purpose = expected.get("surface", "")
        ok = bool(inv.get("files_by_purpose", {}).get(purpose))
        return result(rule, "pass" if ok else "fail", 0.7, f"Surface `{purpose}` {'exists' if ok else 'is missing'}.")
    if ct in {"required_setting", "required_ai_capability", "required_mcp_capability", "required_report", "required_visualization_standard"}:
        keyword = expected.get("name") or expected.get("file") or rule["category"]
        ok = any(keyword and keyword.lower() in f.lower() for f in files)
        return result(rule, "pass" if ok else "unknown", 0.5, f"Heuristic check for `{keyword}` {'found evidence' if ok else 'needs review'}.")
    if ct == "forbidden_dependency_sprawl":
        category = expected.get("category", "")
        matches = [s for s in tech_sprawl if category.replace("_", " ") in s.get("message", "") or category in s.get("message", "")]
        return result(rule, "fail" if matches else "pass", 0.8, "Forbidden dependency sprawl detected." if matches else "No forbidden dependency sprawl detected.", [m["message"] for m in matches], ["standardized dependency category"] if matches else [])
    if ct == "forbidden_missing_metadata":
        missing = [p for p in [".autospec/state/digital-twin.json", ".autospec/state/knowledge-graph.json"] if not (root / p).exists()]
        return result(rule, "fail" if missing else "pass", 0.9, "Metadata missing." if missing else "Required metadata exists.", [], missing)
    return result(rule, "manual_review", 0.2, f"Unsupported v1 check type: {ct}.", rule.get("extraction_evidence", []))


def checks_md(report: dict) -> str:
    lines = ["# Rule Check Results", "", "| Rule | Category | Severity | Status | Summary |", "| --- | --- | --- | --- | --- |"]
    for r in report["results"]:
        lines.append(f"| `{r['rule_id']}` | {r['category']} | {r['severity']} | {r['status']} | {r['summary']} |")
    return "\n".join(lines)


def gap(root: Path) -> int:
    state, reports, backlog = ensure(root)
    checks = load_json(state / "rule-check-results.json", load_json(reports / "rule-check-results.json", {"results": []}))
    results = checks.get("results", [])
    scorecard: dict[str, dict] = {}
    for r in results:
        row = scorecard.setdefault(r["category"], {"required_pass": 0, "required_fail": 0, "partial": 0, "unknown": 0})
        if r["severity"] == "required" and r["status"] == "pass":
            row["required_pass"] += 1
        elif r["severity"] == "required" and r["status"] == "fail":
            row["required_fail"] += 1
        elif r["status"] == "partial":
            row["partial"] += 1
        elif r["status"] in {"unknown", "manual_review"}:
            row["unknown"] += 1
    required_failures = [r for r in results if r["severity"] == "required" and r["status"] == "fail"]
    manual = [r for r in results if r["status"] == "manual_review"]
    waived = [r for r in results if r["status"] in {"waived", "opted_out"}]
    status = "non_compliant" if required_failures else ("manual_review_required" if manual else "compliant")
    gap_report = {"schema": 1, "generated_at": GENERATED_AT, "status": status, "scorecard": scorecard, "required_failures": required_failures, "manual_review_rules": manual, "waived_opted_out_rules": waived, "expired_waivers": [], "top_remediation_candidates": required_failures[:10], "baseline_pack_coverage": {}, "maturity_progress": {}}
    write_json(reports / "constitutional-gap-report-v1.json", gap_report)
    write_text(reports / "constitutional-gap-report-v1.md", gap_md(gap_report))
    maturity_report = maturity(root, results)
    write_json(reports / "maturity-score.json", maturity_report); write_json(state / "maturity-score.json", maturity_report); write_text(reports / "maturity-score.md", maturity_md(maturity_report))
    issue_report = issue_plan_v2(root, results, backlog)
    write_json(reports / "issue-plan-v2.json", issue_report); write_text(reports / "issue-plan-v2.md", issue_v2_md(issue_report))
    print("constitutional gap v1: " + status.upper())
    return 1 if required_failures else 0


def gap_md(rep):
    lines = ["# Constitutional Gap Report v1", "", "## Executive Summary", f"Status: **{rep['status']}**", "", "## Scorecard", "", "| Category | Required pass | Required fail | Partial | Unknown |", "| --- | ---: | ---: | ---: | ---: |"]
    for cat, row in sorted(rep["scorecard"].items()):
        lines.append(f"| {cat} | {row['required_pass']} | {row['required_fail']} | {row['partial']} | {row['unknown']} |")
    lines += ["", "## Required Failures"] + [f"- `{r['rule_id']}` {r['summary']}" for r in rep["required_failures"]] + ["", "## Manual Review Rules"] + [f"- `{r['rule_id']}`" for r in rep["manual_review_rules"]] + ["", "## Waived / Opted-Out Rules"] + [f"- `{r['rule_id']}` ({r['status']})" for r in rep["waived_opted_out_rules"]]
    return "\n".join(lines)


def maturity(root: Path, results: list[dict]) -> dict:
    levels = []
    for level in ["prototype", "production", "enterprise", "autonomous"]:
        eligible = [r for r in results if r["severity"] == "required"]
        passed = [r for r in eligible if r["status"] in {"pass", "waived", "opted_out"}]
        failed = [r for r in eligible if r["status"] == "fail"]
        score = round(len(passed) / len(eligible), 3) if eligible else 0.0
        status = "met" if eligible and not failed else "partial" if passed else "not_met" if failed else "unknown"
        levels.append({"level": level, "status": status, "required_rules_total": len(eligible), "required_rules_passed": len(passed), "required_rules_failed": len(failed), "score": score, "blocking_gaps": [r["rule_id"] for r in failed], "next_actions": [r.get("remediation_hint") or r.get("suggested_issue_title") for r in failed]})
    return {"schema": 1, "generated_at": GENERATED_AT, "repo": root.name, "levels": levels}


def maturity_md(rep):
    lines = ["# Maturity Score", "", "| Level | Status | Score | Required passed | Required failed |", "| --- | --- | ---: | ---: | ---: |"]
    for l in rep["levels"]:
        lines.append(f"| {l['level']} | {l['status']} | {l['score']:.3f} | {l['required_rules_passed']} | {l['required_rules_failed']} |")
    return "\n".join(lines)


def issue_plan_v2(root: Path, results: list[dict], backlog: Path) -> dict:
    if backlog.exists():
        shutil.rmtree(backlog)
    backlog.mkdir(parents=True, exist_ok=True)
    issues = []
    failing = [r for r in results if r["status"] in {"fail", "partial", "unknown", "manual_review"} and r["severity"] in {"required", "recommended"}]
    severity_rank = {"required": 0, "recommended": 1, "optional": 2, "forbidden": 0}
    status_rank = {"fail": 0, "partial": 1, "unknown": 2, "manual_review": 3}
    for i, r in enumerate(sorted(failing, key=lambda x: (severity_rank.get(x["severity"], 9), status_rank.get(x["status"], 9), x["category"], x["rule_id"])), 1):
        issue_id = f"{i:03d}-{re.sub(r'[^a-z0-9]+', '-', r['rule_id'].lower()).strip('-')}"
        title = r.get("suggested_issue_title") or f"feat: satisfy {r['rule_id']}"
        path = backlog / f"{issue_id}.md"
        body = "\n".join([
            f"# {title}", "",
            "## Source Rule", f"- Rule ID: `{r['rule_id']}`", f"- Severity: `{r['severity']}`", f"- Category: `{r['category']}`", "",
            "## Evidence", *(f"- {e}" for e in r.get("evidence", []) or ["No passing evidence found."]), "",
            "## Missing Evidence", *(f"- {e}" for e in r.get("missing_evidence", []) or ["Rule requires manual confirmation."]), "",
            "## Acceptance Criteria", *(f"- [ ] {ac}" for ac in r.get("acceptance_criteria", []) or ["Rule check passes."]), "",
            "## Validation Expectations", "- `bash scripts/autospec-constitution-audit.sh`", "",
            "## Metadata Expectations", "- Rule check results and Digital Twin metadata are refreshed.", "",
            "## Risk Classification Hints", "- Low-risk if limited to docs/tests/metadata; requires guidance if app behavior changes.",
        ])
        write_text(path, body)
        issues.append({"issue_id": issue_id, "title": title, "source_rule_ids": [r["rule_id"]], "severity": r["severity"], "category": r["category"], "maturity_level": "production", "evidence": r.get("evidence", []), "missing_evidence": r.get("missing_evidence", []), "impact": [], "suggested_implementation_mode": "docs/spec/metadata-first", "suggested_worker_eligibility": "needs-classification", "dependencies": [], "acceptance_criteria": r.get("acceptance_criteria", []), "validation_expectations": ["bash scripts/autospec-constitution-audit.sh"], "docs_metadata_expectations": ["refresh rule-check-results and digital twin"], "risk_classification_hints": ["low-risk if metadata/docs only"], "draft_path": f".autospec/backlog/issues-v2/{path.name}"})
    return {"schema": 1, "generated_at": GENERATED_AT, "issues": issues}


def issue_v2_md(rep):
    lines = ["# Issue Plan v2", "", "| Issue | Source rules | Severity | Category |", "| --- | --- | --- | --- |"]
    for i in rep["issues"]:
        lines.append(f"| `{i['issue_id']}` | {', '.join(i['source_rule_ids'])} | {i['severity']} | {i['category']} |")
    return "\n".join(lines)


def audit(root: Path) -> int:
    state, reports, _ = ensure(root)
    commands = []
    script_dir = Path(__file__).resolve().parent
    for cmd in [
        ["bash", str(script_dir / "autospec-constitution-validate.sh"), "--repo-root", str(root)],
        ["bash", str(script_dir / "autospec-baseline-compose.sh"), "--repo-root", str(root)],
        ["bash", str(script_dir / "autospec-build-digital-twin.sh"), "--repo-root", str(root)],
        ["bash", str(script_dir / "autospec-extract-constitution-rules.sh"), "--repo-root", str(root)],
        ["bash", str(script_dir / "autospec-check-rules.sh"), "--repo-root", str(root)],
        ["bash", str(script_dir / "autospec-constitutional-gap-v1.sh"), "--repo-root", str(root)],
    ]:
        cp = subprocess.run(cmd, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        commands.append({"command": " ".join(cmd), "exit_code": cp.returncode})
    checks = load_json(reports / "rule-check-results.json", {"results": []})
    failed = [r for r in checks.get("results", []) if r["severity"] == "required" and r["status"] == "fail"]
    status = "fail" if failed else "pass"
    report = {"schema": 1, "generated_at": GENERATED_AT, "status": status, "commands": commands, "required_failures": [r["rule_id"] for r in failed], "side_effects": {"github_writes": False, "issues_created": False, "network_required": False}}
    write_json(reports / "constitution-audit.json", report)
    write_text(reports / "constitution-audit.md", "# Constitution Audit\n\n## Executive Summary\n\nStatus: **" + status.upper() + "**\n\n## Required Failures\n\n" + "\n".join(f"- `{r}`" for r in report["required_failures"]) + "\n\nNo GitHub writes were performed.")
    print("constitution audit: " + status.upper())
    return 1 if failed else 0


SUPPORTED_CHECK_TYPES = {
    "required_file", "required_directory", "required_capability", "required_tool", "required_dependency",
    "required_metadata", "required_test", "required_doc", "required_surface", "required_setting",
    "required_ai_capability", "required_mcp_capability", "required_report", "required_tutorial",
    "required_visualization_standard", "forbidden_dependency_sprawl", "forbidden_missing_metadata",
    "forbidden_dependency", "forbidden_tool", "manual_review",
}
STRUCTURED_CATEGORIES = {
    "product", "domain", "architecture", "engineering", "testing", "ui", "ux", "accessibility",
    "ai", "rag", "mcp", "nlai", "documentation", "tutorials", "reporting", "analytics",
    "visualization", "security", "privacy", "operations", "diagnostics", "metadata",
    "digital_twin", "onboarding", "modernization", "governance", "data", "docs",
}


def sha_file(path: Path) -> str:
    h = hashlib.sha256()
    h.update(path.read_bytes())
    return h.hexdigest()


def sha_files(paths: list[Path]) -> str:
    h = hashlib.sha256()
    for path in sorted(paths, key=lambda p: p.as_posix()):
        h.update(rel(path.parent.parent if path.parent.name in {"rules", "packs", "manifests", "schemas"} else path.parent, path).encode())
        h.update(b"\0")
        h.update(path.read_bytes())
        h.update(b"\0")
    return h.hexdigest()


def cfg_paths(root: Path) -> tuple[dict, Path | None, Path | None, list[str]]:
    cfg = config(root)
    constitution_cfg = cfg.get("constitution") if isinstance(cfg.get("constitution"), dict) else {}
    baselines_cfg = cfg.get("baselines") if isinstance(cfg.get("baselines"), dict) else {}
    constitution_path = resolve_path(root, constitution_cfg.get("path", "")) if constitution_cfg.get("path") else None
    baselines_path = resolve_path(root, baselines_cfg.get("path", "")) if baselines_cfg.get("path") else None
    profiles = [str(p) for p in baselines_cfg.get("profiles", [])] if isinstance(baselines_cfg.get("profiles", []), list) else []
    return cfg, constitution_path, baselines_path, profiles


def yml_files(root: Path, pattern: str) -> list[Path]:
    if not root or not root.exists():
        return []
    return sorted([p for p in root.glob(pattern) if p.is_file() and p.suffix.lower() in {".yml", ".yaml", ".json"}])


def source_load_data(root: Path) -> dict:
    cfg, constitution_path, baselines_path, profiles = cfg_paths(root)
    def source_entry(kind: str, path: Path | None) -> dict:
        manifests = yml_files(path, "manifests/*") if path else []
        schemas = yml_files(path, "schemas/*.json") if path else []
        rule_files = yml_files(path, "rules/*") if kind == "constitution" and path else []
        pack_files = yml_files(path, "packs/**/*.yml") + yml_files(path, "packs/**/*.yaml") if kind == "baselines" and path else []
        structured = bool(rule_files if kind == "constitution" else pack_files)
        markdown = sorted(path.rglob("*.md")) if path and path.exists() else []
        files = manifests + schemas + rule_files + pack_files
        return {
            "source": "local",
            "path": str(path) if path else "",
            "version": (cfg.get(kind if kind == "constitution" else "baselines") or {}).get("version", "0.1.0") if isinstance(cfg.get(kind if kind == "constitution" else "baselines"), dict) else "0.1.0",
            "manifests_found": [rel(path, p) for p in manifests] if path else [],
            "schemas_found": [rel(path, p) for p in schemas] if path else [],
            "structured_rule_files_found": [rel(path, p) for p in rule_files] if path else [],
            "structured_pack_files_found": [rel(path, p) for p in pack_files] if path else [],
            "structured_available": structured,
            "fallback_used": bool(not structured and markdown),
            "source_hash": sha_files(files) if files else "",
            "warnings": [] if structured else ["Structured policy files missing; Markdown heuristic fallback will be used."],
            "errors": [] if path and path.exists() else [f"{kind} path is missing"],
        }
    return {
        "schema": 1,
        "generated_at": GENERATED_AT,
        "generator": GENERATOR,
        "repo": root.name,
        "constitution": source_entry("constitution", constitution_path),
        "baselines": source_entry("baselines", baselines_path),
        "profiles": profiles,
    }


def policy_load(root: Path) -> int:
    state, reports, _ = ensure(root)
    data = source_load_data(root)
    write_json(state / "policy-sources.json", data)
    write_json(reports / "policy-source-load.json", data)
    lines = ["# Policy Source Load", "", "## Constitution", f"- Path: `{data['constitution']['path']}`", f"- Structured: {data['constitution']['structured_available']}", f"- Fallback used: {data['constitution']['fallback_used']}", "", "## Baselines", f"- Path: `{data['baselines']['path']}`", f"- Structured: {data['baselines']['structured_available']}", f"- Fallback used: {data['baselines']['fallback_used']}", "", "## Warnings"]
    warnings = data["constitution"]["warnings"] + data["baselines"]["warnings"]
    lines += [f"- {w}" for w in warnings] or ["- None."]
    lines += ["", "## Errors"] + ([f"- {e}" for e in data["constitution"]["errors"] + data["baselines"]["errors"]] or ["- None."])
    write_text(reports / "policy-source-load.md", "\n".join(lines))
    print("policy source load: PASS")
    return 0


def structured_rule_items(path: Path) -> list[dict]:
    data = load_json(path, {}) if path.suffix.lower() == ".json" else load_yaml(path)
    items = data.get("rules") if isinstance(data, dict) else []
    return items if isinstance(items, list) else []


def validate_policy_sources(root: Path) -> int:
    _, reports, _ = ensure(root)
    cfg, constitution_path, baselines_path, profiles = cfg_paths(root)
    findings: list[dict] = []
    def add(code, message, severity="error", path=""):
        findings.append({"code": code, "severity": severity, "message": message, "path": path, "action": "Fix the structured policy source metadata or document a follow-up."})

    # Constitution validation.
    c_cats = set()
    c_levels = set()
    c_rule_ids: dict[str, str] = {}
    if not constitution_path or not constitution_path.exists():
        add("CONSTITUTION_PATH_MISSING", "configured constitution path is missing")
    else:
        for relp in ["manifests/constitution.yml", "manifests/doctrines.yml", "manifests/categories.yml", "manifests/maturity-levels.yml"]:
            if not (constitution_path / relp).exists():
                add("CONSTITUTION_MANIFEST_MISSING", f"missing {relp}", path=relp)
        cats = load_yaml(constitution_path / "manifests/categories.yml")
        c_cats = set(cats.get("categories", [])) if isinstance(cats.get("categories"), list) else STRUCTURED_CATEGORIES
        levels = load_yaml(constitution_path / "manifests/maturity-levels.yml")
        c_levels = set((levels.get("levels") or {}).keys()) or set(MATURITY)
        manifest = load_yaml(constitution_path / "manifests/constitution.yml")
        for item in manifest.get("doctrines", []) if isinstance(manifest.get("doctrines"), list) else []:
            for key in ["document", "rules"]:
                if item.get(key) and not (constitution_path / item[key]).exists():
                    add("CONSTITUTION_REFERENCE_MISSING", f"doctrine {item.get('id')} references missing {key}: {item[key]}", path=item[key])
        for schema in yml_files(constitution_path, "schemas/*.json"):
            load_json(schema, None) if schema.exists() else add("SCHEMA_MISSING", f"missing schema {rel(constitution_path, schema)}")
        for rule_file in yml_files(constitution_path, "rules/*"):
            for raw in structured_rule_items(rule_file):
                rid = raw.get("rule_id") or raw.get("id")
                if not rid:
                    add("RULE_FIELD_MISSING", f"rule missing id in {rel(constitution_path, rule_file)}", path=rel(constitution_path, rule_file)); continue
                if rid in c_rule_ids:
                    add("DUPLICATE_RULE_ID", f"duplicate rule id {rid}", path=rel(constitution_path, rule_file))
                c_rule_ids[rid] = rel(constitution_path, rule_file)
                for field in ["title", "summary", "source", "category", "severity", "maturity", "check", "evidence_required", "acceptance_criteria", "remediation", "risk"]:
                    if not raw.get(field):
                        add("RULE_FIELD_MISSING", f"rule {rid} missing {field}", path=rel(constitution_path, rule_file))
                if raw.get("category") not in c_cats:
                    add("UNKNOWN_CATEGORY", f"rule {rid} uses unknown category {raw.get('category')}", path=rel(constitution_path, rule_file))
                if raw.get("severity") not in {"required", "recommended", "optional", "forbidden"}:
                    add("UNKNOWN_SEVERITY", f"rule {rid} uses unknown severity {raw.get('severity')}", path=rel(constitution_path, rule_file))
                if (raw.get("maturity") or {}).get("level") not in c_levels:
                    add("UNKNOWN_MATURITY", f"rule {rid} uses unknown maturity level {(raw.get('maturity') or {}).get('level')}", path=rel(constitution_path, rule_file))
                ct = (raw.get("check") or {}).get("type") or raw.get("check_type")
                if ct not in SUPPORTED_CHECK_TYPES:
                    add("UNSUPPORTED_CHECK_TYPE", f"rule {rid} uses unsupported check type {ct}", "warning", rel(constitution_path, rule_file))

    # Baseline validation.
    if not baselines_path or not baselines_path.exists():
        add("BASELINES_PATH_MISSING", "configured baselines path is missing")
    else:
        b_manifest_path = baselines_path / "manifests/baselines.yml"
        p_manifest_path = baselines_path / "manifests/profiles.yml"
        if not b_manifest_path.exists():
            add("BASELINES_MANIFEST_MISSING", "missing manifests/baselines.yml")
        if not p_manifest_path.exists():
            add("BASELINE_PROFILES_MANIFEST_MISSING", "missing manifests/profiles.yml")
        manifest = load_yaml(b_manifest_path)
        pack_map = {p.get("id"): p.get("file") for p in manifest.get("packs", []) if isinstance(p, dict)}
        for pid, file in sorted(pack_map.items()):
            if not file or not (baselines_path / file).exists():
                add("BASELINE_PACK_MISSING", f"pack {pid} references missing file {file}", path=file or "")
        profiles_manifest = load_yaml(p_manifest_path)
        profile_map = {p.get("id"): p.get("packs", []) for p in profiles_manifest.get("profiles", []) if isinstance(p, dict)}
        for profile in profiles:
            if profile not in profile_map:
                add("BASELINE_PROFILE_MISSING", f"requested profile is missing: {profile}")
        rule_ids = set()
        for pack_file in yml_files(baselines_path, "packs/**/*.yml") + yml_files(baselines_path, "packs/**/*.yaml"):
            data = load_yaml(pack_file)
            pid = data.get("id", rel(baselines_path, pack_file))
            for dep in (data.get("inherits") or []) + (data.get("requires") or []) + (data.get("conflicts_with") or []):
                if dep not in pack_map:
                    add("BASELINE_PACK_REFERENCE_UNKNOWN", f"pack {pid} references unknown pack {dep}", path=rel(baselines_path, pack_file))
            for field in ["id", "title", "version", "type", "summary", "applies_when", "capabilities", "metadata_required", "rules", "quality_gates", "issue_templates"]:
                if field not in data:
                    add("BASELINE_PACK_FIELD_MISSING", f"pack {pid} missing {field}", path=rel(baselines_path, pack_file))
            for raw in data.get("rules", []) if isinstance(data.get("rules"), list) else []:
                rid = raw.get("rule_id") or raw.get("id")
                if rid in rule_ids:
                    add("DUPLICATE_RULE_ID", f"duplicate baseline rule id {rid}", path=rel(baselines_path, pack_file))
                rule_ids.add(rid)
                for field in ["title", "summary", "source", "category", "severity", "maturity", "check", "evidence_required", "acceptance_criteria", "remediation", "risk"]:
                    if not raw.get(field):
                        add("RULE_FIELD_MISSING", f"rule {rid} missing {field}", path=rel(baselines_path, pack_file))
                ct = (raw.get("check") or {}).get("type") or raw.get("check_type")
                if ct not in SUPPORTED_CHECK_TYPES:
                    add("UNSUPPORTED_CHECK_TYPE", f"rule {rid} uses unsupported check type {ct}", "warning", rel(baselines_path, pack_file))

    report = {"schema": 1, "generated_at": GENERATED_AT, "status": "fail" if any(f["severity"] == "error" for f in findings) else "pass", "findings": sorted(findings, key=lambda f: (f["severity"], f["code"], f["message"]))}
    write_json(reports / "policy-source-validation.json", report)
    lines = ["# Policy Source Validation", "", f"Status: **{report['status'].upper()}**", "", "| Severity | Code | Message | Action |", "| --- | --- | --- | --- |"]
    lines += [f"| {f['severity']} | `{f['code']}` | {f['message']} | {f['action']} |" for f in report["findings"]] or ["| info | `OK` | No findings. | None. |"]
    write_text(reports / "policy-source-validation.md", "\n".join(lines))
    print("policy source validation: " + report["status"].upper())
    return 1 if report["status"] == "fail" else 0


def lock_policy_sources(root: Path) -> int:
    _, reports, _ = ensure(root)
    data = source_load_data(root)
    cfg, constitution_path, baselines_path, profiles = cfg_paths(root)
    c_files = yml_files(constitution_path, "manifests/*") + yml_files(constitution_path, "rules/*") if constitution_path else []
    b_files = yml_files(baselines_path, "manifests/*") + yml_files(baselines_path, "packs/**/*.yml") + yml_files(baselines_path, "packs/**/*.yaml") if baselines_path else []
    lock = {
        "schema": 1,
        "generated_at": GENERATED_AT,
        "constitution": {
            "source": "local", "path": (cfg.get("constitution") or {}).get("path", ""), "version": (cfg.get("constitution") or {}).get("version", "0.1.0"),
            "manifest_hash": sha_files(yml_files(constitution_path, "manifests/*")) if constitution_path else "", "rules_hash": sha_files(yml_files(constitution_path, "rules/*")) if constitution_path else "",
            "files": [{"path": rel(constitution_path, p), "sha256": sha_file(p)} for p in sorted(c_files)] if constitution_path else [],
        },
        "baselines": {
            "source": "local", "path": (cfg.get("baselines") or {}).get("path", ""), "profiles": profiles,
            "manifest_hash": sha_files(yml_files(baselines_path, "manifests/*")) if baselines_path else "", "packs_hash": sha_files(yml_files(baselines_path, "packs/**/*.yml") + yml_files(baselines_path, "packs/**/*.yaml")) if baselines_path else "",
            "files": [{"path": rel(baselines_path, p), "sha256": sha_file(p)} for p in sorted(b_files)] if baselines_path else [],
        },
    }
    write_json(root / ".autospec" / "policy-sources.lock.json", lock)
    write_json(reports / "policy-source-lock.json", lock)
    write_text(reports / "policy-source-lock.md", "# Policy Source Lock\n\n- Constitution files: %d\n- Baseline files: %d\n- Lockfile: `.autospec/policy-sources.lock.json`" % (len(lock["constitution"]["files"]), len(lock["baselines"]["files"])))
    print("policy source lock: PASS")
    return 0


def normalize_structured_rule(raw: dict, source_type: str, source_repo: str, source_file: str, *, source_pack: str = "", profile: str = "", version: str = "0.1.0") -> dict:
    check = raw.get("check") if isinstance(raw.get("check"), dict) else {}
    remediation = raw.get("remediation") if isinstance(raw.get("remediation"), dict) else {}
    source = raw.get("source") if isinstance(raw.get("source"), dict) else {}
    applies = raw.get("applies_when") if isinstance(raw.get("applies_when"), dict) else {}
    maturity = raw.get("maturity") if isinstance(raw.get("maturity"), dict) else {}
    rid = str(raw.get("rule_id") or raw.get("id") or f"{source_type}.{slug(source_file)}.{slug(raw.get('title','rule'))}")
    return {
        "schema": 1, "rule_id": rid, "title": str(raw.get("title") or rid), "summary": str(raw.get("summary") or ""),
        "source_type": source_type, "source_format": "structured_yaml", "source_repo": source_repo, "source_file": source_file,
        "source_doctrine": source.get("doctrine", ""), "source_pack": source_pack or source.get("baseline", ""), "source_heading": source.get("section", ""),
        "profile": str(raw.get("profile") or profile or ""), "version": version, "maturity_level": str(maturity.get("level") or raw.get("maturity_level") or "production"),
        "severity": str(raw.get("severity") or "recommended"), "category": str(raw.get("category") or "metadata"),
        "applies_when": applies, "check_type": str(check.get("type") or raw.get("check_type") or "manual_review"),
        "expected": check.get("expected") if isinstance(check.get("expected"), dict) else (raw.get("expected") if isinstance(raw.get("expected"), dict) else {}),
        "acceptance_criteria": [str(x) for x in raw.get("acceptance_criteria", [])] if isinstance(raw.get("acceptance_criteria"), list) else [],
        "evidence_required": [str(x) for x in raw.get("evidence_required", [])] if isinstance(raw.get("evidence_required"), list) else [],
        "metadata_required": [str(x) for x in raw.get("metadata_required", [])] if isinstance(raw.get("metadata_required"), list) else [],
        "quality_gates": [str(x) for x in raw.get("quality_gates", [])] if isinstance(raw.get("quality_gates"), list) else [],
        "remediation_hint": str(remediation.get("hint") or raw.get("remediation_hint") or ""),
        "suggested_issue_title": str(remediation.get("suggested_issue_title") or ""),
        "suggested_labels": [str(x) for x in remediation.get("suggested_labels", [])] if isinstance(remediation.get("suggested_labels"), list) else [],
        "risk": raw.get("risk") if isinstance(raw.get("risk"), dict) else {},
        "confidence": 1.0,
        "extraction_evidence": [source_file],
    }


def structured_constitution_rules(repo: Path) -> list[dict]:
    if not repo or not repo.exists():
        return []
    rules = []
    version = str(load_yaml(repo / "manifests/constitution.yml").get("version", "0.1.0"))
    for path in yml_files(repo, "rules/*"):
        for raw in structured_rule_items(path):
            if isinstance(raw, dict):
                rules.append(normalize_structured_rule(raw, "constitution", "autospec-constitution", rel(repo, path), version=version))
    return rules


def baseline_pack_map(repo: Path) -> dict[str, dict]:
    if not repo or not repo.exists():
        return {}
    packs = {}
    manifest = load_yaml(repo / "manifests/baselines.yml")
    for item in manifest.get("packs", []) if isinstance(manifest.get("packs"), list) else []:
        path = repo / item.get("file", "")
        data = load_yaml(path)
        if data:
            data["_path"] = rel(repo, path); packs[data.get("id") or item.get("id")] = data
    if not packs:
        for path in yml_files(repo, "packs/**/*.yml") + yml_files(repo, "packs/**/*.yaml"):
            data = load_yaml(path)
            if data.get("id"):
                data["_path"] = rel(repo, path); packs[data["id"]] = data
    return packs


def selected_pack_ids(repo: Path, profiles: list[str]) -> list[str]:
    profiles_manifest = load_yaml(repo / "manifests/profiles.yml") if repo else {}
    profile_map = {p.get("id"): p.get("packs", []) for p in profiles_manifest.get("profiles", []) if isinstance(p, dict)}
    ordered = []
    def add(pid):
        if pid and pid not in ordered:
            ordered.append(pid)
    for profile in profiles:
        for pid in profile_map.get(profile, []):
            add(pid)
    packs = baseline_pack_map(repo)
    changed = True
    while changed:
        changed = False
        for pid in list(ordered):
            pack = packs.get(pid, {})
            for dep in (pack.get("inherits") or []) + (pack.get("requires") or []):
                if dep not in ordered:
                    ordered.append(dep); changed = True
    return sorted(ordered)


def structured_baseline_rules(repo: Path, profiles: list[str]) -> list[dict]:
    packs = baseline_pack_map(repo)
    version = str(load_yaml(repo / "manifests/baselines.yml").get("version", "0.1.0")) if repo else "0.1.0"
    pack_profiles = {}
    profiles_manifest = load_yaml(repo / "manifests/profiles.yml") if repo else {}
    for profile in profiles_manifest.get("profiles", []) if isinstance(profiles_manifest.get("profiles"), list) else []:
        for pid in profile.get("packs", []) or []:
            pack_profiles.setdefault(pid, []).append(profile.get("id"))
    rules = []
    for pid in selected_pack_ids(repo, profiles):
        pack = packs.get(pid, {})
        profile = next((p for p in profiles if p in pack_profiles.get(pid, [])), "")
        for raw in pack.get("rules", []) if isinstance(pack.get("rules"), list) else []:
            rules.append(normalize_structured_rule(raw, "baseline", "autospec-baselines", pack.get("_path", ""), source_pack=pid, profile=profile, version=version))
        for cap in (pack.get("capabilities") or {}).get("required", []) if isinstance(pack.get("capabilities"), dict) else []:
            cid = slug(str(cap)).replace("_", "-")
            raw = {
                "id": f"baseline.{slug(pid)}.capability.{slug(str(cap))}",
                "title": f"Capability required: {cap}",
                "summary": f"Baseline pack {pid} requires capability: {cap}.",
                "source": {"baseline": pid, "section": "capabilities.required"},
                "category": "metadata", "severity": "required", "maturity": {"level": "production"},
                "applies_when": {"profiles": [profile] if profile else []},
                "check": {"type": "required_capability", "expected": {"id": cid}},
                "evidence_required": [str(cap)], "acceptance_criteria": [f"Capability `{cap}` is detected or documented."],
                "metadata_required": pack.get("metadata_required", []), "quality_gates": pack.get("quality_gates", []),
                "remediation": {"hint": f"Add or document capability: {cap}.", "suggested_issue_title": f"feat: add {cap}", "suggested_labels": ["autospec:baseline"]},
                "risk": {"level": "medium", "requires_human_review": True, "requires_architecture_review": False},
            }
            rules.append(normalize_structured_rule(raw, "baseline", "autospec-baselines", pack.get("_path", ""), source_pack=pid, profile=profile, version=version))
    return rules


def generic_structured_rules(repo: Path, source_type: str, profiles: list[str]) -> list[dict]:
    """Legacy-compatible structured reader for top-level rules arrays outside manifests."""
    if not repo or not repo.exists():
        return []
    out = []
    source_repo = "autospec-constitution" if source_type == "constitution" else "autospec-baselines"
    for path in sorted(repo.rglob("*")):
        if not path.is_file() or path.suffix.lower() not in {".yml", ".yaml", ".json"}:
            continue
        if "manifests" in path.parts:
            continue
        data = load_json(path, {}) if path.suffix.lower() == ".json" else load_yaml(path)
        if not isinstance(data.get("rules"), list):
            continue
        profile = next((p for p in profiles if p in path.parts), "")
        for raw in data.get("rules", []):
            if isinstance(raw, dict):
                out.append(normalize_structured_rule(raw, source_type, source_repo, rel(repo, path), profile=profile))
    return out


def markdown_rule_v2(repo: Path, path: Path, source_type: str) -> list[dict]:
    rules = markdown_rules(repo, path, source_type)
    for rule in rules:
        rule["source_format"] = "markdown_heuristic"; rule["source_repo"] = repo.name; rule["source_doctrine"] = ""; rule["source_pack"] = ""; rule["version"] = "unknown"; rule["confidence"] = min(float(rule.get("confidence", 0.35)), 0.45)
    return rules


def extract(root: Path) -> int:  # v2 override
    state, reports, _ = ensure(root)
    cfg, constitution_path, baseline_path, profiles = cfg_paths(root)
    constitution_rules = structured_constitution_rules(constitution_path) if constitution_path else []
    baseline_rules = structured_baseline_rules(baseline_path, profiles) if baseline_path else []
    if not constitution_rules and constitution_path:
        constitution_rules = generic_structured_rules(constitution_path, "constitution", profiles)
    if not baseline_rules and baseline_path:
        baseline_rules = generic_structured_rules(baseline_path, "baseline", profiles)
    if not constitution_rules and constitution_path and constitution_path.exists():
        for path in sorted(constitution_path.rglob("*.md")):
            constitution_rules.extend(markdown_rule_v2(constitution_path, path, "constitution"))
    if not baseline_rules and baseline_path and baseline_path.exists():
        for path in sorted(baseline_path.rglob("*.md")):
            baseline_rules.extend(markdown_rule_v2(baseline_path, path, "baseline"))
    constitution_rules = unique_rules(constitution_rules)
    baseline_rules = unique_rules(baseline_rules)
    write_json(state / "constitution-rules.json", {"schema": 1, "generated_at": GENERATED_AT, "generator": GENERATOR, "repo": root.name, "rules": constitution_rules})
    write_json(state / "baseline-rules.json", {"schema": 1, "generated_at": GENERATED_AT, "generator": GENERATOR, "repo": root.name, "rules": baseline_rules})
    effective, waiver_report = resolve_effective(root, constitution_rules, baseline_rules)
    write_json(state / "effective-rules.json", effective); write_json(reports / "effective-rules.json", effective)
    structured_count = len([r for r in constitution_rules + baseline_rules if r.get("source_format") == "structured_yaml"])
    heuristic_count = len([r for r in constitution_rules + baseline_rules if r.get("source_format") == "markdown_heuristic" or not r.get("source_format")])
    write_json(reports / "rule-extraction.json", {"schema": 1, "generated_at": GENERATED_AT, "constitution_rules": len(constitution_rules), "baseline_rules": len(baseline_rules), "structured_rules": structured_count, "heuristic_rules": heuristic_count, "waiver_findings": waiver_report["findings"]})
    write_text(reports / "rule-extraction.md", extraction_md(constitution_rules, baseline_rules, waiver_report) + f"\n\n## structured vs heuristic\n\n- Structured rules: {structured_count}\n- Heuristic rules: {heuristic_count}\n")
    write_text(reports / "effective-rules.md", effective_md(effective)); write_text(reports / "rule-waivers.md", waivers_md(waiver_report))
    print("rule extraction: PASS")
    return 0


def applies_list(value, key):
    if isinstance(value, dict):
        raw = value.get(key, [])
        return raw if isinstance(raw, list) else ([raw] if raw else [])
    if isinstance(value, list):
        vals = []
        for item in value:
            if isinstance(item, dict) and key in item:
                vals.append(item[key])
        return vals
    return []


def resolve_effective(root: Path, constitution_rules: list[dict], baseline_rules: list[dict]) -> tuple[dict, dict]:  # v2 override
    cfg = config(root)
    profiles = set(cfg.get("baselines", {}).get("profiles", []) if isinstance(cfg.get("baselines"), dict) else [])
    app_type = str(cfg.get("application", {}).get("type", "") if isinstance(cfg.get("application"), dict) else "")
    target = str(cfg.get("application", {}).get("maturity_target", "production") if isinstance(cfg.get("application"), dict) else "production")
    tech_names, _ = load_tech_names(root)
    waiver_data = waivers(root); waiver_by_rule = {w.get("rule_id"): w for w in waiver_data.get("waivers", []) if w.get("rule_id")}
    opt_out_caps = {o.get("capability"): o for o in waiver_data.get("opt_outs", []) if o.get("capability")}
    all_rules = constitution_rules + baseline_rules
    findings = waiver_findings(waiver_data, {r["rule_id"] for r in all_rules})
    effective_rules = []
    seen = {}
    for rule in all_rules:
        resolution = "active"
        aw = rule.get("applies_when", {})
        if rule["rule_id"] in seen:
            resolution = "conflict"
        elif not rule.get("rule_id") or not rule.get("title"):
            resolution = "invalid"
        elif rule["rule_id"] in waiver_by_rule:
            resolution = "waived"
        elif rule.get("check_type") not in SUPPORTED_CHECK_TYPES:
            resolution = "unsupported_check_type"
        elif applies_list(aw, "application_types") and app_type not in applies_list(aw, "application_types"):
            resolution = "inactive_application_type"
        elif applies_list(aw, "profiles") and profiles and not (profiles & set(map(str, applies_list(aw, "profiles")))):
            resolution = "inactive_profile_mismatch"
        elif applies_list(aw, "technologies") and tech_names and not (tech_names & set(map(str, applies_list(aw, "technologies")))):
            resolution = "inactive_technology_mismatch"
        elif MATURITY.get(rule.get("maturity_level", "production"), 2) > MATURITY.get(target, 2):
            resolution = "inactive_maturity_level"
        elif rule.get("expected", {}).get("id") in opt_out_caps or rule.get("expected", {}).get("capability") in opt_out_caps:
            resolution = "opted_out"
        elif rule.get("check_type") == "manual_review":
            resolution = "manual_review"
        item = dict(rule); item["resolution"] = resolution; item["waiver"] = waiver_by_rule.get(rule["rule_id"])
        effective_rules.append(item); seen[rule["rule_id"]] = item
    return {"schema": 1, "generated_at": GENERATED_AT, "generator": GENERATOR, "repo": root.name, "profiles": sorted(profiles), "application_type": app_type, "maturity_target": target, "rules": sorted(effective_rules, key=lambda r: r["rule_id"]), "waiver_findings": findings}, {"data": waiver_data, "findings": findings}


def result(rule, status, confidence, summary, evidence=None, missing=None):  # v2 override
    return {
        "rule_id": rule["rule_id"], "title": rule["title"], "source_file": rule.get("source_file", ""), "source_format": rule.get("source_format", "markdown_heuristic"),
        "source_repo": rule.get("source_repo", ""), "source_doctrine": rule.get("source_doctrine", ""), "source_pack": rule.get("source_pack", ""),
        "category": rule["category"], "severity": rule["severity"], "status": status, "confidence": confidence, "summary": summary,
        "evidence": evidence or [], "missing_evidence": missing or [], "affected_metadata": rule.get("metadata_required", []),
        "suggested_issue_title": "" if status in {"pass", "waived", "opted_out"} else (rule.get("suggested_issue_title") or f"feat: satisfy {rule['rule_id']}"),
        "suggested_labels": rule.get("suggested_labels", []),
        "acceptance_criteria": rule.get("acceptance_criteria", []), "quality_gates": rule.get("quality_gates", []),
        "remediation_hint": rule.get("remediation_hint", ""), "risk": rule.get("risk", {}),
    }


def file_exists_any(root: Path, paths: list[str]) -> tuple[bool, list[str]]:
    hits = []
    for p in paths:
        if p and (root / p).exists():
            hits.append(p)
    return bool(hits), hits


def check_rule(root: Path, rule: dict, inv: dict, caps: dict, tech_names: set[str], tech_sprawl: list[dict]):  # v2 override
    resolution = rule.get("resolution")
    if resolution in {"waived", "opted_out"}:
        return result(rule, resolution, 1.0, f"Rule is {resolution}.", [json.dumps(rule.get("waiver"), sort_keys=True)] if rule.get("waiver") else [])
    if resolution in {"inactive_profile_mismatch", "inactive_maturity_level", "inactive_application_type", "inactive_technology_mismatch", "inactive_repo_condition"}:
        return result(rule, "not_applicable", 0.9, f"Rule inactive: {resolution}.")
    if resolution in {"unsupported_check_type", "invalid"}:
        return result(rule, "manual_review", 0.2, f"Rule requires engine support or repair: {resolution}.", rule.get("extraction_evidence", []))
    if resolution == "manual_review" or rule.get("check_type") == "manual_review":
        return result(rule, "manual_review", rule.get("confidence", 0.3), "Rule requires manual interpretation.", rule.get("extraction_evidence", []))
    ct = rule.get("check_type")
    expected = rule.get("expected", {}) if isinstance(rule.get("expected"), dict) else {}
    files = {f["path"] for f in inv.get("files", []) if isinstance(f, dict) and f.get("path")}
    all_text = "\n".join(sorted(files)).lower()
    if ct == "required_file":
        paths = expected.get("paths") if isinstance(expected.get("paths"), list) else [expected.get("file") or expected.get("path") or ""]
        ok, hits = file_exists_any(root, [str(p) for p in paths])
        return result(rule, "pass" if ok else "fail", 0.9, "Required file evidence exists." if ok else "Required file evidence is missing.", hits, [] if ok else [str(p) for p in paths if p])
    if ct in {"required_directory"}:
        paths = expected.get("paths") if isinstance(expected.get("paths"), list) else [expected.get("directory") or expected.get("path") or ""]
        hits = [str(p) for p in paths if p and (root / str(p)).is_dir()]
        return result(rule, "pass" if hits else "fail", 0.9, "Required directory exists." if hits else "Required directory is missing.", hits, [] if hits else [str(p) for p in paths if p])
    if ct == "required_metadata":
        name = expected.get("metadata") or expected.get("file") or expected.get("path")
        candidates = [str(name)] if str(name).startswith(".autospec/") else [f".autospec/state/{name}.json", f".autospec/state/{name}.yml", f".autospec/state/{name}.md"]
        ok, hits = file_exists_any(root, candidates)
        return result(rule, "pass" if ok else "fail", 0.9, f"Required metadata `{name}` {'exists' if ok else 'is missing'}.", hits, [] if ok else candidates)
    if ct in {"required_tool", "required_dependency"}:
        name = expected.get("name") or expected.get("tool") or expected.get("dependency")
        ok = str(name) in tech_names or str(name).lower() in all_text
        return result(rule, "pass" if ok else "fail", 0.85, f"Required tool/dependency `{name}` {'was found' if ok else 'was not found'}.", [str(name)] if ok else [], [str(name)] if not ok else [])
    if ct in {"forbidden_dependency", "forbidden_tool"}:
        name = expected.get("name") or expected.get("tool") or expected.get("dependency")
        found = str(name) in tech_names or str(name).lower() in all_text
        return result(rule, "fail" if found else "pass", 0.85, f"Forbidden tool/dependency `{name}` {'was found' if found else 'was not found'}.", [str(name)] if found else [], [])
    if ct in {"required_capability", "required_ai_capability", "required_mcp_capability"}:
        cid = expected.get("id") or expected.get("capability") or expected.get("name")
        cap_ids = {c.get("id") for c in caps.get("capabilities", []) if isinstance(c, dict)}
        ok = cid in cap_ids
        return result(rule, "pass" if ok else "fail", 0.8, f"Required capability `{cid}` {'was found' if ok else 'was not found'}.", [str(cid)] if ok else [], [str(cid)] if not ok else [])
    if ct in {"required_doc", "required_tutorial"}:
        docs = inv.get("files_by_purpose", {}).get("documentation", [])
        return result(rule, "pass" if docs else "fail", 0.75, "Documentation evidence exists." if docs else "Documentation evidence is missing.", docs, [] if docs else ["documentation"])
    if ct == "required_test":
        tests = inv.get("files_by_purpose", {}).get("test", [])
        return result(rule, "pass" if tests else "fail", 0.75, "Test evidence exists." if tests else "Test evidence is missing.", tests, [] if tests else ["test"])
    if ct == "required_surface":
        surface = expected.get("surface", "ui")
        paths = inv.get("files_by_purpose", {}).get(str(surface), [])
        return result(rule, "pass" if paths else "fail", 0.7, f"Surface `{surface}` {'exists' if paths else 'is missing'}.", paths, [] if paths else [str(surface)])
    if ct in {"required_setting", "required_report", "required_visualization_standard"}:
        keywords = expected.get("keywords") if isinstance(expected.get("keywords"), list) else [expected.get("name") or expected.get("file") or rule["category"]]
        ok = any(str(k).lower() in all_text for k in keywords if k)
        return result(rule, "pass" if ok else "unknown", 0.5, "Heuristic evidence found." if ok else "Heuristic check needs review.", [str(k) for k in keywords if str(k).lower() in all_text], [str(k) for k in keywords if k] if not ok else [])
    if ct == "forbidden_dependency_sprawl":
        cats = expected.get("categories") if isinstance(expected.get("categories"), list) else [expected.get("category", "")]
        matches = [s for s in tech_sprawl if any(str(c).replace("_", " ") in s.get("message", "") or str(c) in s.get("message", "") for c in cats)]
        return result(rule, "fail" if matches else "pass", 0.8, "Forbidden dependency sprawl detected." if matches else "No forbidden dependency sprawl detected.", [m["message"] for m in matches], ["standardized dependency category"] if matches else [])
    if ct == "forbidden_missing_metadata":
        missing = [p for p in [".autospec/state/digital-twin.json", ".autospec/state/knowledge-graph.json"] if not (root / p).exists()]
        return result(rule, "fail" if missing else "pass", 0.9, "Metadata missing." if missing else "Required metadata exists.", [], missing)
    return result(rule, "manual_review", 0.2, f"Unsupported check type: {ct}.", rule.get("extraction_evidence", []))


def quality_gate_report(root: Path, results: list[dict]) -> dict:
    gates = []
    for r in results:
        for idx, gate in enumerate(r.get("quality_gates", []) or []):
            ok = r["status"] == "pass"
            gates.append({"id": f"{r['rule_id']}.gate_{idx+1}", "title": str(gate), "source_rule_id": r["rule_id"], "category": r["category"], "required_evidence": r.get("evidence_required", []), "status": "pass" if ok else ("manual_review" if r["status"] == "manual_review" else "fail"), "evidence": r.get("evidence", []), "missing_evidence": r.get("missing_evidence", []) or ([str(gate)] if not ok else [])})
    return {"schema": 1, "generated_at": GENERATED_AT, "gates": sorted(gates, key=lambda g: g["id"])}


def quality_gates_md(rep: dict) -> str:
    lines = ["# Quality Gates", "", "| Gate | Source rule | Status | Missing evidence |", "| --- | --- | --- | --- |"]
    lines += [f"| {g['title']} | `{g['source_rule_id']}` | {g['status']} | {', '.join(g.get('missing_evidence', []))} |" for g in rep["gates"]] or ["| None |  |  |  |"]
    return "\n".join(lines)


def check(root: Path) -> int:  # v2 override
    state, reports, _ = ensure(root)
    effective = load_json(state / "effective-rules.json", {"rules": []})
    inv = load_json(state / "repository-inventory.json", {"files": [], "files_by_purpose": {}})
    caps = load_json(state / "capability-registry.json", {"capabilities": []})
    tech_names, tech_sprawl = load_tech_names(root)
    results = [check_rule(root, rule, inv, caps, tech_names, tech_sprawl) for rule in effective.get("rules", [])]
    report = {"schema": 1, "generated_at": GENERATED_AT, "generator": GENERATOR, "repo": root.name, "results": sorted(results, key=lambda r: r["rule_id"])}
    write_json(reports / "rule-check-results.json", report); write_json(state / "rule-check-results.json", report); write_text(reports / "rule-check-results.md", checks_md(report))
    gates = quality_gate_report(root, results)
    write_json(state / "quality-gates.json", gates); write_json(reports / "quality-gates.json", gates); write_text(reports / "quality-gates.md", quality_gates_md(gates))
    failed = any(r["status"] in {"fail", "partial", "unknown"} and r["severity"] == "required" for r in results)
    print("rule checks: FAIL" if failed else "rule checks: PASS")
    return 1 if failed else 0


def issue_plan_v3(root: Path, results: list[dict]) -> dict:
    backlog = root / ".autospec/backlog/issues-v3"
    if backlog.exists():
        shutil.rmtree(backlog)
    backlog.mkdir(parents=True, exist_ok=True)
    failing = [r for r in results if r["status"] in {"fail", "partial", "unknown", "manual_review"} and r["severity"] in {"required", "recommended"}]
    issues = []
    for i, r in enumerate(sorted(failing, key=lambda x: (x["category"], x.get("source_pack", ""), x["rule_id"])), 1):
        title = r.get("suggested_issue_title") or f"feat: satisfy {r['rule_id']}"
        issue_id = f"{i:03d}-{re.sub(r'[^a-z0-9]+', '-', title.lower()).strip('-')[:72]}"
        path = backlog / f"{issue_id}.md"
        lines = [
            f"# {title}", "", "## Source", f"- Rule ID: `{r['rule_id']}`", f"- Doctrine: `{r.get('source_doctrine') or 'n/a'}`", f"- Baseline pack: `{r.get('source_pack') or 'n/a'}`", f"- Source file: `{r.get('source_file')}`", f"- Severity: `{r['severity']}`", f"- Maturity: `production`", f"- Category: `{r['category']}`", "",
            "## Evidence", *(f"- {e}" for e in r.get("evidence", []) or ["No passing evidence found."]), "",
            "## Missing Evidence", *(f"- {e}" for e in r.get("missing_evidence", []) or ["Manual review needed."]), "",
            "## Remediation", r.get("remediation_hint") or "Satisfy the structured rule.", "",
            "## Suggested Labels", *(f"- {l}" for l in r.get("suggested_labels", []) or ["autospec:discovered"]), "",
            "## Acceptance Criteria", *(f"- [ ] {ac}" for ac in r.get("acceptance_criteria", []) or ["Rule check passes."]), "",
            "## Quality Gates", *(f"- [ ] {g}" for g in r.get("quality_gates", []) or ["Relevant quality gates pass or are waived."]), "",
            "## Validation", "- `bash scripts/autospec-constitution-audit.sh`", "",
            "## Metadata Expectations", "- Refresh rule-check results, quality gates, and Digital Twin metadata.",
        ]
        write_text(path, "\n".join(lines))
        issues.append({"issue_id": issue_id, "title": title, "source_rule_ids": [r["rule_id"]], "source_doctrine": r.get("source_doctrine", ""), "source_baseline_pack": r.get("source_pack", ""), "source_file": r.get("source_file", ""), "rule_severity": r["severity"], "maturity_level": "production", "category": r["category"], "evidence": r.get("evidence", []), "missing_evidence": r.get("missing_evidence", []), "remediation_hint": r.get("remediation_hint", ""), "suggested_labels": r.get("suggested_labels", []), "acceptance_criteria": r.get("acceptance_criteria", []), "quality_gates": r.get("quality_gates", []), "implementation_mode": "docs/spec/metadata-first", "worker_eligibility": "needs-classification", "risk": r.get("risk", {}), "dependencies": [], "validation_expectations": ["bash scripts/autospec-constitution-audit.sh"], "metadata_expectations": ["refresh rule-check-results and quality-gates"], "draft_path": f".autospec/backlog/issues-v3/{path.name}"})
    return {"schema": 1, "generated_at": GENERATED_AT, "issues": issues}


def issue_v3_md(rep: dict) -> str:
    lines = ["# Issue Plan v3", "", "| Issue | Source rules | Source | Severity | Category |", "| --- | --- | --- | --- | --- |"]
    lines += [f"| `{i['issue_id']}` | {', '.join(i['source_rule_ids'])} | {i.get('source_baseline_pack') or i.get('source_doctrine') or i.get('source_file')} | {i['rule_severity']} | {i['category']} |" for i in rep["issues"]] or ["| None |  |  |  |  |"]
    return "\n".join(lines)


def gap(root: Path) -> int:  # v2 override
    state, reports, _ = ensure(root)
    checks = load_json(state / "rule-check-results.json", load_json(reports / "rule-check-results.json", {"results": []}))
    results = checks.get("results", [])
    code = _old_gap(root) if False else None
    scorecard: dict[str, dict] = {}
    for r in results:
        row = scorecard.setdefault(r["category"], {"required_pass": 0, "required_fail": 0, "partial": 0, "unknown": 0})
        if r["severity"] == "required" and r["status"] == "pass": row["required_pass"] += 1
        elif r["severity"] == "required" and r["status"] == "fail": row["required_fail"] += 1
        elif r["status"] == "partial": row["partial"] += 1
        elif r["status"] in {"unknown", "manual_review"}: row["unknown"] += 1
    required_failures = [r for r in results if r["severity"] == "required" and r["status"] == "fail"]
    manual = [r for r in results if r["status"] == "manual_review"]; waived = [r for r in results if r["status"] in {"waived", "opted_out"}]
    status = "non_compliant" if required_failures else ("manual_review_required" if manual else "compliant")
    gap_report = {"schema": 1, "generated_at": GENERATED_AT, "status": status, "scorecard": scorecard, "required_failures": required_failures, "manual_review_rules": manual, "waived_opted_out_rules": waived, "expired_waivers": [], "top_remediation_candidates": required_failures[:10], "baseline_pack_coverage": {}, "maturity_progress": {}}
    write_json(reports / "constitutional-gap-report-v1.json", gap_report); write_json(reports / "constitutional-gap-report-v1.md.json", gap_report)
    write_text(reports / "constitutional-gap-report-v1.md", gap_md(gap_report))
    maturity_report = maturity(root, results)
    write_json(reports / "maturity-score.json", maturity_report); write_json(state / "maturity-score.json", maturity_report); write_text(reports / "maturity-score.md", maturity_md(maturity_report))
    issue_v2 = issue_plan_v2(root, results, root / ".autospec/backlog/issues-v2")
    write_json(reports / "issue-plan-v2.json", issue_v2); write_text(reports / "issue-plan-v2.md", issue_v2_md(issue_v2))
    issue_v3 = issue_plan_v3(root, results)
    write_json(reports / "issue-plan-v3.json", issue_v3); write_text(reports / "issue-plan-v3.md", issue_v3_md(issue_v3))
    print("constitutional gap v1: " + status.upper())
    return 1 if required_failures else 0


def policy_compatibility(root: Path) -> int:
    _, reports, _ = ensure(root)
    effective = load_json(root / ".autospec/state/effective-rules.json", {"rules": []})
    validation = load_json(reports / "policy-source-validation.json", {"findings": []})
    unsupported = [r for r in effective.get("rules", []) if r.get("check_type") not in SUPPORTED_CHECK_TYPES]
    findings = validation.get("findings", []) + [{"code": "UNSUPPORTED_CHECK_TYPE", "message": f"unsupported check type {r.get('check_type')} for {r['rule_id']}", "severity": "warning"} for r in unsupported]
    report = {"schema": 1, "generated_at": GENERATED_AT, "status": "warn" if findings else "pass", "unsupported_check_types": sorted({r.get("check_type") for r in unsupported}), "findings": findings, "recommended_engine_follow_up_issues": [f"feat: add engine support for {t}" for t in sorted({r.get("check_type") for r in unsupported})]}
    write_json(reports / "policy-compatibility.json", report)
    lines = ["# Policy Compatibility", "", f"Status: **{report['status'].upper()}**", "", "## Findings"] + ([f"- `{f.get('code')}`: {f.get('message')}" for f in findings] or ["- None."]) + ["", "## Recommended Engine Follow-up Issues"] + ([f"- {i}" for i in report["recommended_engine_follow_up_issues"]] or ["- None."])
    write_text(reports / "policy-compatibility.md", "\n".join(lines))
    print("policy compatibility: " + report["status"].upper())
    return 0


def audit(root: Path) -> int:  # v2 override
    _, reports, _ = ensure(root)
    script_dir = Path(__file__).resolve().parent
    commands = []
    for cmd in [
        ["bash", str(script_dir / "autospec-validate-policy-sources.sh"), "--repo-root", str(root)],
        ["bash", str(script_dir / "autospec-lock-policy-sources.sh"), "--repo-root", str(root)],
        ["bash", str(script_dir / "autospec-build-digital-twin.sh"), "--repo-root", str(root)],
        ["bash", str(script_dir / "autospec-baseline-compose.sh"), "--repo-root", str(root)],
        ["bash", str(script_dir / "autospec-extract-constitution-rules.sh"), "--repo-root", str(root)],
        ["bash", str(script_dir / "autospec-check-rules.sh"), "--repo-root", str(root)],
        ["bash", str(script_dir / "autospec-constitutional-gap-v1.sh"), "--repo-root", str(root)],
        ["bash", str(script_dir / "autospec-policy-compatibility.sh"), "--repo-root", str(root)],
    ]:
        cp = subprocess.run(cmd, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        commands.append({"command": " ".join(cmd), "exit_code": cp.returncode})
    extraction = load_json(reports / "rule-extraction.json", {})
    checks = load_json(reports / "rule-check-results.json", {"results": []})
    results = checks.get("results", [])
    required_failed = [r for r in results if r["severity"] == "required" and r["status"] == "fail"]
    counts = {s: len([r for r in results if r["status"] == s]) for s in sorted({r["status"] for r in results})}
    status = "fail" if required_failed else "pass"
    report = {"schema": 1, "generated_at": GENERATED_AT, "status": status, "commands": commands, "structured_rules": extraction.get("structured_rules", 0), "heuristic_rules": extraction.get("heuristic_rules", 0), "active_rules": len([r for r in load_json(root / ".autospec/state/effective-rules.json", {"rules": []}).get("rules", []) if r.get("resolution") == "active"]), "result_counts": counts, "required_failures": [r["rule_id"] for r in required_failed], "side_effects": {"github_writes": False, "issues_created": False, "network_required": False}}
    write_json(reports / "constitution-audit.json", report)
    md = ["# Constitution Audit", "", "## Executive Summary", f"Status: **{status.upper()}**", f"- structured vs heuristic: {report['structured_rules']} structured / {report['heuristic_rules']} heuristic", f"- Active rules: {report['active_rules']}", f"- Result counts: {json.dumps(counts, sort_keys=True)}", "", "## Top Required Failures"] + ([f"- `{r}`" for r in report["required_failures"][:10]] or ["- None."]) + ["", "## Issue Plan v3", "- See `.autospec/reports/issue-plan-v3.md`.", "", "No GitHub writes were performed."]
    write_text(reports / "constitution-audit.md", "\n".join(md))
    print("constitution audit: " + status.upper())
    return 1 if required_failed else 0

def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["load", "validate-sources", "lock-sources", "extract", "check", "gap", "audit", "compatibility"])
    parser.add_argument("--repo-root", default=os.getcwd())
    args = parser.parse_args()
    root = Path(args.repo_root).resolve()
    if args.command == "load":
        return policy_load(root)
    if args.command == "validate-sources":
        return validate_policy_sources(root)
    if args.command == "lock-sources":
        return lock_policy_sources(root)
    if args.command == "extract":
        return extract(root)
    if args.command == "check":
        return check(root)
    if args.command == "gap":
        return gap(root)
    if args.command == "audit":
        return audit(root)
    if args.command == "compatibility":
        return policy_compatibility(root)
    return 2


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Constitution rule interpretation v1.

Local/read-only by default. No GitHub writes, no network, no issue publishing.
"""

from __future__ import annotations

import argparse
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["extract", "check", "gap", "audit"])
    parser.add_argument("--repo-root", default=os.getcwd())
    args = parser.parse_args()
    root = Path(args.repo_root).resolve()
    if args.command == "extract":
        return extract(root)
    if args.command == "check":
        return check(root)
    if args.command == "gap":
        return gap(root)
    if args.command == "audit":
        return audit(root)
    return 2


if __name__ == "__main__":
    sys.exit(main())

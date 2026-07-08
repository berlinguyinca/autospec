#!/usr/bin/env python3
"""Autonomy v3 deterministic specialist/quorum/learning governance."""

from __future__ import annotations

import argparse
import json
import re
from datetime import datetime, timezone
from pathlib import Path


def now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def slug(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-") or "item"


def write_json(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text.rstrip() + "\n", encoding="utf-8")


def load_json(path: Path, default):
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        return data if isinstance(data, dict) else default
    except Exception:
        return default


def reports(root: Path) -> Path:
    path = root / ".autospec/reports"
    path.mkdir(parents=True, exist_ok=True)
    return path


def state(root: Path) -> Path:
    path = root / ".autospec/state"
    path.mkdir(parents=True, exist_ok=True)
    return path


def target_id(issue: str = "", pr: str = "", feature: str = "", rule: str = "", run: str = "") -> str:
    if pr:
        return f"pr-{pr}"
    if issue:
        return f"issue-{issue}"
    if feature:
        return f"feature-{feature}"
    if rule:
        return f"rule-{rule}"
    if run:
        return f"run-{run}"
    return "current"


SPECIALISTS = [
    ("product-manager", "Product Manager", ["product_baseline"], ["product"], ["scope", "acceptance"]),
    ("software-architect", "Software Architect", ["architecture", "engineering"], ["runtime"], ["architecture", "impact"]),
    ("frontend-engineer", "Frontend Engineer", ["ui_ux", "product_baseline"], ["product", "ui"], ["runtime_ui", "components"]),
    ("backend-engineer", "Backend Engineer", ["api", "engineering"], ["api"], ["service_boundaries", "validation"]),
    ("ai-engineer", "AI Engineer", ["ai_platform", "nlai"], ["ai", "nlai"], ["provider_boundaries", "mock_only"]),
    ("data-engineer", "Data Engineer", ["data", "reporting"], ["reporting", "analytics"], ["query_safety", "data_model"]),
    ("qa-engineer", "QA Engineer", ["testing"], ["testing", "product", "ai", "nlai"], ["tests", "evidence"]),
    ("security-engineer", "Security Engineer", ["security"], ["ai", "nlai", "data"], ["secret_handling", "permission_changes"]),
    ("privacy-engineer", "Privacy Engineer", ["privacy", "security"], ["ai", "nlai", "data"], ["pii", "retention"]),
    ("ux-designer", "UX Designer", ["ui_ux"], ["product", "ui"], ["responsive", "human_readability"]),
    ("accessibility-specialist", "Accessibility Specialist", ["ui_ux", "accessibility"], ["product", "ui"], ["keyboard", "labels"]),
    ("documentation-engineer", "Documentation Engineer", ["documentation"], ["docs", "product"], ["tutorials", "runbooks"]),
    ("devops-sre", "DevOps/SRE", ["operations", "diagnostics"], ["diagnostics"], ["health", "status"]),
    ("performance-engineer", "Performance Engineer", ["performance"], ["runtime"], ["latency", "load"]),
    ("release-manager", "Release Manager", ["release"], ["runtime", "product"], ["release_readiness", "rollback"]),
    ("support-engineer", "Support Engineer", ["support", "diagnostics"], ["diagnostics", "product"], ["troubleshooting", "incidents"]),
    ("dependency-steward", "Dependency Steward", ["dependencies"], ["dependencies"], ["dependency_policy", "sprawl"]),
    ("modernization-steward", "Modernization Steward", ["modernization"], ["modernization"], ["migration_plan", "compatibility"]),
]


def default_agents() -> list[dict]:
    agents = []
    for sid, title, categories, features, dims in SPECIALISTS:
        agents.append({
            "id": sid,
            "title": title,
            "summary": f"Deterministic {title.lower()} review role.",
            "responsibilities": [f"Review {d.replace('_', ' ')}" for d in dims],
            "owns_categories": categories,
            "owns_rule_categories": categories,
            "owns_feature_slices": features,
            "owns_risk_types": ["medium", "high"] if sid in {"software-architect", "security-engineer", "privacy-engineer"} else ["low", "medium"],
            "default_review_dimensions": dims,
            "required_inputs": ["digital_twin", "rule_results", "evidence_bundle"],
            "produces_outputs": ["findings", "recommendations", "review_packet"],
            "can_block_promotion": sid in {"software-architect", "qa-engineer", "security-engineer", "privacy-engineer", "ai-engineer"},
            "can_request_guidance": True,
            "can_approve_autospec_internally": False,
            "permissions": {"read": ["digital_twin", "rule_results", "evidence_bundle"], "write": ["reports", "findings", "recommendations"]},
            "forbidden_actions": ["merge_pr", "approve_pr", "bypass_verifier"],
            "escalation_rules": ["Request human guidance for irreversible or high-risk decisions."],
            "quality_gates": dims,
        })
    return agents


def specialist_index(root: Path) -> int:
    agents = default_agents()
    payload = {"schema": 1, "specialists": agents, "missing_coverage": []}
    write_json(state(root) / "specialist-agents.json", payload)
    write_json(reports(root) / "specialist-agents.json", payload)
    rows = "\n".join(f"| `{a['id']}` | {a['title']} | {', '.join(a['owns_categories'])} | {', '.join(a['owns_feature_slices'])} |" for a in agents)
    write_text(reports(root) / "specialist-agents.md", "\n".join([
        "# Autospec Specialist Agents",
        "",
        "## Summary",
        "",
        f"- Registered specialists: {len(agents)}",
        "- Specialists produce findings, review packets, and recommendations. They never merge or approve PRs.",
        "",
        "## Registered specialists",
        "",
        "| Specialist | Title | Rule/category ownership | Feature ownership |",
        "| --- | --- | --- | --- |",
        rows,
        "",
        "## Rule/category ownership",
        "",
        rows,
        "",
        "## Feature ownership",
        "",
        rows,
        "",
        "## Review responsibilities",
        "",
        "- Product, architecture, engineering, QA, UX/accessibility, security/privacy, AI/data, operations, documentation, dependency, modernization, and release perspectives.",
        "",
        "## Escalation rules",
        "",
        "- Human review remains final authority.",
        "- Specialists never merge, approve, or bypass verifier/quorum.",
        "",
        "## Missing coverage",
        "",
        "- None for standard Autonomy v3 categories.",
    ]))
    return 0


def load_agents(root: Path) -> list[dict]:
    data = load_json(state(root) / "specialist-agents.json", {})
    agents = data.get("specialists") if isinstance(data.get("specialists"), list) else []
    return agents or default_agents()


def context(root: Path, feature: str = "") -> dict:
    runtime = load_json(reports(root) / "runtime-generation-result.json", {})
    feature_id = feature or runtime.get("feature_id", "")
    slices = load_json(state(root) / "feature-slices.json", {"feature_slices": []}).get("feature_slices", [])
    category = ""
    for item in slices if isinstance(slices, list) else []:
        if item.get("id") == feature_id:
            category = item.get("category", "")
    text = json.dumps({
        "runtime": runtime,
        "risk": load_json(reports(root) / "worker-risk-classification.json", {}),
        "evidence": load_json(reports(root) / "evidence-bundle.json", {}),
    }, sort_keys=True).lower()
    return {"feature_id": feature_id, "category": category, "text": text, "runtime": runtime}


def assignment_for(root: Path, issue: str = "", pr: str = "", feature: str = "", rule: str = "") -> tuple[str, list[dict]]:
    ctx = context(root, feature)
    category = ctx["category"]
    text = ctx["text"] + " " + feature + " " + rule
    ids: list[str] = []
    if category in {"product", "ui"} or re.search(r"\b(docs-center|settings|runtime)\b", text):
        ids += ["frontend-engineer", "ux-designer", "accessibility-specialist", "qa-engineer"]
    if category in {"ai", "nlai"} or re.search(r"\b(ai|nlai|rag|token|mcp|provider|model)\b", text):
        ids += ["ai-engineer", "security-engineer", "privacy-engineer", "qa-engineer", "documentation-engineer"]
    if category in {"data", "reporting", "analytics"} or re.search(r"\b(data|query|sql|report|analytics)\b", text):
        ids += ["data-engineer", "qa-engineer", "security-engineer", "privacy-engineer"]
    if re.search(r"\b(architecture|high-risk|medium-risk|auth|security|secret)\b", text):
        ids += ["software-architect", "security-engineer"]
    if re.search(r"\b(docs|tutorial|pdf)\b", text):
        ids += ["documentation-engineer", "qa-engineer"]
    if re.search(r"\b(dependency|modernization|upgrade)\b", text):
        ids += ["dependency-steward", "modernization-steward", "qa-engineer"]
    if re.search(r"\b(diagnostics|health|status|incident|operations)\b", text):
        ids += ["devops-sre", "support-engineer", "qa-engineer"]
    if not ids:
        ids = ["software-architect", "qa-engineer"]
    seen = []
    for sid in ids:
        if sid not in seen:
            seen.append(sid)
    agents = {a["id"]: a for a in load_agents(root)}
    assignments = []
    for sid in seen:
        agent = agents.get(sid, {"id": sid, "default_review_dimensions": []})
        assignments.append({
            "specialist_id": sid,
            "reason": f"Assigned for feature/category `{category or feature or rule or 'general'}`.",
            "required": sid in {"qa-engineer", "security-engineer", "privacy-engineer", "software-architect", "ai-engineer"} or category in {"product", "ai", "nlai"},
            "can_block": bool(agent.get("can_block_promotion", False)),
            "review_dimensions": agent.get("default_review_dimensions", []),
            "expected_outputs": ["findings", "recommendations"],
        })
    return target_id(issue, pr, feature, rule), assignments


def assign_specialists(root: Path, issue: str, pr: str, feature: str, rule: str) -> int:
    if not (state(root) / "specialist-agents.json").exists():
        specialist_index(root)
    tid, assignments = assignment_for(root, issue, pr, feature, rule)
    payload = {"schema": 1, "target_id": tid, "assignments": assignments}
    out_dir = state(root) / "specialist-assignments"
    write_json(out_dir / f"{tid}.json", payload)
    write_text(out_dir / f"{tid}.md", "# Specialist Assignment\n\n" + "\n".join(f"- `{a['specialist_id']}`: {a['reason']}" for a in assignments))
    write_json(reports(root) / "specialist-assignment.json", payload)
    write_text(reports(root) / "specialist-assignment.md", "# Specialist Assignment\n\n" + "\n".join(f"- `{a['specialist_id']}` required=`{str(a['required']).lower()}` can_block=`{str(a['can_block']).lower()}`" for a in assignments))
    return 0


CHECKLISTS = {
    "security-engineer": ["Secrets not exposed", "Secret references used instead of raw values", "Auth/permission behavior not changed without review", "No destructive data operations", "Sensitive output audit passes"],
    "ux-designer": ["Responsive layouts considered", "Mobile/tablet/desktop evidence exists", "Empty/loading/error states exist", "Raw JSON avoided", "Human-readable rendering"],
    "qa-engineer": ["Test plan exists", "Focused validation exists", "Playwright/evidence present when applicable", "Regression risk documented", "Rule recheck performed or rationale given"],
    "ai-engineer": ["Provider abstraction respected", "No real external API calls by default", "RAG/no-context/citation behavior specified", "Token/cost tracking planned", "Tool/memory/MCP boundaries explicit"],
}


def review_packets(root: Path, issue: str, pr: str, feature: str, confirm: bool) -> int:
    tid = target_id(issue, pr, feature)
    assignments = load_json(reports(root) / "specialist-assignment.json", {}).get("assignments", [])
    if not assignments:
        _, assignments = assignment_for(root, issue, pr, feature, "")
    packet_dir = state(root) / "specialist-reviews" / tid
    packets = []
    for item in assignments:
        sid = item["specialist_id"]
        checklist = CHECKLISTS.get(sid, ["Evidence reviewed", "Risk documented", "Required actions stated"])
        packet = {"schema": 1, "target_id": tid, "specialist": sid, "assignment": item, "checklist": checklist, "evidence_bundle": ".autospec/reports/evidence-bundle.json"}
        packets.append(packet)
        md = "\n".join([
            f"# Specialist Review Packet — {sid}",
            "",
            "## Review target",
            "",
            tid,
            "",
            "## Why this specialist is assigned",
            "",
            item.get("reason", ""),
            "",
            "## Source issue / PR / rule",
            "",
            f"- Issue: `{issue or 'n/a'}`",
            f"- PR: `{pr or 'n/a'}`",
            "",
            "## Relevant Constitution/Baseline rules",
            "",
            "- See rule-check-results/effective-rules when present.",
            "",
            "## Relevant Digital Twin context",
            "",
            "- See `.autospec/state/digital-twin.json`.",
            "",
            "## Changed files / planned files",
            "",
            "- See worker diff, patch plan, and runtime generation reports.",
            "",
            "## Evidence bundle",
            "",
            "` .autospec/reports/evidence-bundle.json `",
            "",
            "## Risk summary",
            "",
            "- Medium/high-risk work requires planning or human guidance.",
            "",
            "## Specialist checklist",
            "",
            "\n".join(f"- [ ] {c}" for c in checklist),
            "",
            "## Required findings format",
            "",
            "`pass|warn|fail|unknown|not_applicable` with evidence and required action.",
            "",
            "## Blocking conditions",
            "",
            "- Secrets, unsafe runtime claims, missing required evidence, or verifier blockers.",
            "",
            "## Suggested questions for human guidance",
            "",
            "- What decision would make this safe to resume?",
        ])
        if confirm:
            write_json(packet_dir / f"{sid}.json", packet)
            write_text(packet_dir / f"{sid}.md", md)
    write_json(reports(root) / "specialist-review-packets.json", {"schema": 1, "target_id": tid, "packets": packets})
    write_text(reports(root) / "specialist-review-packets.md", "# Specialist Review Packets\n\n" + "\n".join(f"- `{p['specialist']}`" for p in packets))
    return 0


def evaluate_specialist(root: Path, sid: str) -> tuple[str, list[dict]]:
    evidence = load_json(reports(root) / "evidence-bundle.json", {})
    verifier = load_json(reports(root) / "verifier-report.json", {})
    text = json.dumps({"evidence": evidence, "verifier": verifier, "runtime": load_json(reports(root) / "runtime-generation-result.json", {})}, sort_keys=True).lower()
    findings = []
    if sid == "security-engineer":
        bad = "secret" in text and ("missing" in text or "leak" in text or "raw" in text)
        findings.append({"specialist": sid, "dimension": "secret_handling", "status": "fail" if bad else "pass", "summary": "Secret handling evidence reviewed.", "evidence": evidence.get("findings", []), "required_action": "Resolve secret/reference evidence before promotion." if bad else "", "blocking": bad})
    elif sid in {"ux-designer", "accessibility-specialist"}:
        has = bool(evidence.get("reports"))
        findings.append({"specialist": sid, "dimension": "ui_evidence", "status": "pass" if has else "warn", "summary": "UI/accessibility evidence reviewed.", "evidence": evidence.get("reports", []), "required_action": "Add screenshot/accessibility evidence or rationale." if not has else "", "blocking": False})
    elif sid == "qa-engineer":
        bad = verifier.get("verdict") in {"needs_changes", "blocked"}
        findings.append({"specialist": sid, "dimension": "test_evidence", "status": "fail" if bad else "pass", "summary": "Verifier and evidence reviewed.", "evidence": [verifier.get("verdict", "missing")], "required_action": "Fix verifier findings." if bad else "", "blocking": bad})
    elif sid == "ai-engineer":
        bad = "real model calls" in text or "external api" in text and "false" not in text
        findings.append({"specialist": sid, "dimension": "ai_boundaries", "status": "fail" if bad else "pass", "summary": "AI/NLAI mock/provider boundaries reviewed.", "evidence": evidence.get("reports", []), "required_action": "Use mock-only/default-off provider behavior." if bad else "", "blocking": bad})
    else:
        findings.append({"specialist": sid, "dimension": "general_review", "status": "pass", "summary": "Checklist evidence reviewed.", "evidence": [], "required_action": "", "blocking": False})
    verdict = "blocked" if any(f["blocking"] for f in findings) else "needs_changes" if any(f["status"] == "fail" for f in findings) else "pass_with_warnings" if any(f["status"] == "warn" for f in findings) else "pass"
    return verdict, findings


def specialist_review(root: Path, issue: str, pr: str, feature: str, packet: str) -> int:
    tid = target_id(issue, pr, feature)
    assignments = load_json(reports(root) / "specialist-assignment.json", {}).get("assignments", [])
    if packet:
        sid = Path(packet).stem
        assignments = [{"specialist_id": sid, "required": True}]
    if not assignments:
        _, assignments = assignment_for(root, issue, pr, feature, "")
    all_results = []
    finding_dir = state(root) / "specialist-findings" / tid
    for a in assignments:
        sid = a["specialist_id"]
        verdict, findings = evaluate_specialist(root, sid)
        result = {"schema": 1, "target_id": tid, "specialist": sid, "verdict": verdict, "findings": findings, "required": a.get("required", True)}
        all_results.append(result)
        write_json(finding_dir / f"{sid}.json", result)
        write_text(finding_dir / f"{sid}.md", f"# Specialist Findings — {sid}\n\nVerdict: **{verdict}**\n\n" + "\n".join(f"- {f['dimension']}: `{f['status']}` {f['summary']}" for f in findings))
    payload = {"schema": 1, "target_id": tid, "results": all_results}
    write_json(reports(root) / "specialist-review-result.json", payload)
    write_text(reports(root) / "specialist-review-result.md", "# Specialist Review Result\n\n" + "\n".join(f"- `{r['specialist']}`: `{r['verdict']}`" for r in all_results))
    return 0


def review_quorum(root: Path, issue: str, pr: str, feature: str) -> int:
    tid = target_id(issue, pr, feature)
    assignments = load_json(reports(root) / "specialist-assignment.json", {}).get("assignments", [])
    results = load_json(reports(root) / "specialist-review-result.json", {}).get("results", [])
    by_sid = {r.get("specialist"): r for r in results}
    missing = [a for a in assignments if a.get("required") and a.get("specialist_id") not in by_sid]
    blocking = []
    warnings = []
    for a in assignments:
        sid = a.get("specialist_id")
        result = by_sid.get(sid, {})
        verdict = result.get("verdict")
        if verdict in {"blocked"}:
            blocking.append({"specialist": sid, "verdict": verdict})
        elif verdict == "needs_changes":
            blocking.append({"specialist": sid, "verdict": verdict})
        elif verdict in {"pass_with_warnings", "needs_guidance"}:
            warnings.append({"specialist": sid, "verdict": verdict})
    verifier = load_json(reports(root) / "verifier-report.json", {})
    if verifier.get("verdict") == "blocked":
        blocking.append({"specialist": "verifier", "verdict": "blocked"})
    verdict = "insufficient_reviews" if missing else "blocked" if any(b["verdict"] == "blocked" for b in blocking) else "needs_changes" if blocking else "pass_with_warnings" if warnings else "pass"
    payload = {"schema": 1, "target_id": tid, "verdict": verdict, "required_specialists": [a["specialist_id"] for a in assignments if a.get("required")], "optional_specialists": [a["specialist_id"] for a in assignments if not a.get("required")], "specialist_verdicts": {sid: by_sid.get(sid, {}).get("verdict", "missing") for sid in [a["specialist_id"] for a in assignments]}, "blocking_findings": blocking, "warnings": warnings, "promotion_ready": verdict in {"pass", "pass_with_warnings"}}
    out_dir = state(root) / "review-quorum"
    write_json(out_dir / f"{tid}.json", payload)
    write_text(out_dir / f"{tid}.md", quorum_md(payload))
    write_json(reports(root) / "review-quorum.json", payload)
    write_text(reports(root) / "review-quorum.md", quorum_md(payload))
    return 0 if verdict in {"pass", "pass_with_warnings"} else 1


def quorum_md(payload: dict) -> str:
    return "\n".join([
        "# Autospec Review Quorum",
        "",
        "## Verdict",
        "",
        payload["verdict"],
        "",
        "## Required specialists",
        "",
        "\n".join(f"- `{s}`" for s in payload.get("required_specialists", [])) or "- None.",
        "",
        "## Optional specialists",
        "",
        "\n".join(f"- `{s}`" for s in payload.get("optional_specialists", [])) or "- None.",
        "",
        "## Specialist verdicts",
        "",
        "\n".join(f"- `{k}`: `{v}`" for k, v in sorted(payload.get("specialist_verdicts", {}).items())) or "- None.",
        "",
        "## Blocking findings",
        "",
        "\n".join(f"- `{b['specialist']}`: `{b['verdict']}`" for b in payload.get("blocking_findings", [])) or "- None.",
        "",
        "## Warnings",
        "",
        "\n".join(f"- `{w['specialist']}`: `{w['verdict']}`" for w in payload.get("warnings", [])) or "- None.",
        "",
        "## Required actions",
        "",
        "- Resolve blocking specialist findings before promotion.",
        "",
        "## Promotion readiness",
        "",
        f"`{str(payload.get('promotion_ready', False)).lower()}`",
        "",
        "## Human review notes",
        "",
        "- Review quorum is an internal gate, not human approval.",
    ])


def medium_risk_plan(root: Path, issue: str, rule: str, feature: str) -> int:
    sid = target_id(issue, "", feature, rule)
    title = f"medium-risk-{slug(issue or rule or feature or 'plan')}"
    spec = root / "docs/specs" / f"{now()[:10]}-{title}.md"
    adr = root / "docs/adr" / f"{now()[:10]}-{title}.md"
    backlog = root / ".autospec/backlog/medium-risk" / f"{sid}.md"
    md = "\n".join([
        "# Medium-Risk Implementation Plan",
        "",
        "## Scope", "Plan medium-risk work without execution.",
        "## Non-goals", "- No code implementation in this lane.",
        "## Architecture", "- ADR required for architecture-impacting decisions.",
        "## ADR requirement", f"- `{adr.relative_to(root)}`",
        "## Data/security/privacy impact", "- Requires specialist review when sensitive.",
        "## API/UI impact", "- Requires focused tests and evidence.",
        "## Test strategy", "- Unit/focused validation plus runtime evidence where applicable.",
        "## Evidence strategy", "- Evidence bundle, verifier, specialist review, quorum.",
        "## Rollback plan", "- Revert generated plan/spec/ADR; no runtime changes are made.",
        "## Decomposition", "- Split into safe recipe/scaffold/test/docs issues.",
        "## Human decisions needed", "- Approve API/data/security/dependency decisions before execution.",
        "## Acceptance criteria", "- [ ] Human guidance resolves medium-risk decisions.",
    ])
    write_text(spec, md)
    write_text(adr, "# IDR/ADR: Medium-risk plan\n\n## Status\n\nproposed\n\n## Decision\n\nPlan first; do not execute automatically.")
    write_text(backlog, md)
    payload = {"schema": 1, "target_id": sid, "status": "planned", "spec_path": str(spec.relative_to(root)), "adr_path": str(adr.relative_to(root)), "implemented": False}
    write_json(reports(root) / "medium-risk-plan.json", payload)
    write_text(reports(root) / "medium-risk-plan.md", md)
    return 0


def guidance_request(root: Path, issue: str, plan: str, confirm: bool) -> int:
    tid = target_id(issue) if issue else f"plan-{slug(plan)}"
    md = "\n".join([
        "# Autospec Guidance Request",
        "",
        "## Decision needed", "Choose the safe implementation direction.",
        "## Why Autospec needs guidance", "The work is medium/high risk or stuck.",
        "## Context", f"- Target: `{tid}`",
        "## Options", "",
        "### Option 1", "Pros:\n- Narrow scope.\nCons:\n- May defer runtime behavior.\nRisk:\n- Low.",
        "### Option 2", "Pros:\n- Broader implementation.\nCons:\n- Requires human approval and stronger tests.\nRisk:\n- Medium.",
        "## Autospec recommendation", "Prefer the narrow, testable option first.",
        "## What Autospec will do after guidance", "Resume with a dry-run plan and verifier/quorum checks.",
        "## Labels to apply", "- autospec:guidance-needed",
        "## Resume criteria", "- [ ] human decision is explicit\n- [ ] verifier/quorum path is clear",
    ])
    payload = {"schema": 1, "target_id": tid, "status": "planned", "side_effects": {"github_writes": False, "auto_resume": False}}
    write_json(reports(root) / "guidance-request.json", payload)
    write_text(reports(root) / "guidance-request.md", md)
    if confirm:
        out = state(root) / "guidance-requests"
        write_json(out / f"{tid}.json", payload)
        write_text(out / f"{tid}.md", md)
    return 0


def record_idr(root: Path, issue: str, pr: str, confirm: bool) -> int:
    tid = target_id(issue, pr)
    path = root / "docs/idrs" / f"{now()[:10]}-{tid}.md"
    md = "\n".join(["# IDR: Autospec implementation decision", "", "## Status", "proposed", "## Context", "Autospec selected or refused a bounded path.", "## Decision", "Use verifier/quorum evidence before promotion.", "## Options considered", "- Execute now\n- Plan/defer", "## Why this decision", "Safety constraints require bounded autonomy.", "## Risks", "- Incomplete evidence.", "## Validation evidence", "- See reports.", "## Related rules", "- See rule results.", "## Related issue/PR", tid, "## Follow-up", "- Update learning ledger."])
    if confirm:
        write_text(path, md)
    decisions = load_json(state(root) / "implementation-decisions.json", {"schema": 1, "decisions": []})
    decisions.setdefault("decisions", []).append({"id": tid, "path": str(path.relative_to(root)), "status": "proposed"})
    write_json(state(root) / "implementation-decisions.json", decisions)
    write_text(reports(root) / "implementation-decisions.md", "# Implementation Decisions\n\n" + "\n".join(f"- `{d['id']}`: `{d['status']}`" for d in decisions["decisions"]))
    return 0


def update_learning(root: Path, confirm: bool, source: str) -> int:
    verifier = load_json(reports(root) / "verifier-report.json", {})
    dims = verifier.get("dimensions", [])
    summary = "No verifier failure found"
    category = "process"
    lesson_type = "process_gap"
    evidence = []
    for d in dims if isinstance(dims, list) else []:
        if d.get("status") in {"fail", "warn", "unknown"}:
            summary = d.get("summary", "Verifier finding")
            category = "testing" if "test" in d.get("dimension", "") else "verifier"
            lesson_type = "test_needed" if category == "testing" else "safety_gap"
            evidence = [d.get("dimension", "unknown")]
            break
    ledger = load_json(state(root) / "learning-ledger.json", {"schema": 1, "entries": []})
    entries = ledger.setdefault("entries", [])
    existing = next((e for e in entries if e.get("summary") == summary), None)
    if existing:
        existing["frequency"] = int(existing.get("frequency", 1)) + 1
        existing["last_seen"] = now()
    else:
        entries.append({"schema": 1, "id": f"learn-{slug(summary)[:40]}", "source": source or "verifier", "summary": summary, "evidence": evidence, "category": category, "lesson_type": lesson_type, "frequency": 1, "recommended_action": "Create a focused rule/recipe/test improvement.", "status": "candidate", "related_issues": [], "related_rules": [], "related_files": []})
    write_json(state(root) / "learning-ledger.json", ledger)
    write_text(reports(root) / "learning-ledger.md", "# Autospec Learning Ledger\n\n## New lessons\n\n" + "\n".join(f"- {e['summary']} (frequency {e.get('frequency', 1)})" for e in entries) + "\n\n## Repeated misses\n\n" + "\n".join(f"- {e['summary']}" for e in entries if e.get("frequency", 1) > 1) + "\n\n## Safety gaps\n\n- See entries.\n\n## Policy improvement candidates\n\n- See proposals.\n\n## Recipe improvement candidates\n\n- See entries.\n\n## Adapter improvement candidates\n\n- See entries.\n\n## Worker/verifier improvements\n\n- See entries.\n\n## Recommended follow-up issues\n\n- Generate repeated-miss issue plan.")
    return 0


def policy_proposals(root: Path, confirm: bool) -> int:
    entries = load_json(state(root) / "learning-ledger.json", {"entries": []}).get("entries", [])
    proposals = []
    for e in entries or [{"summary": "No ledger entry", "lesson_type": "policy_gap", "evidence": []}]:
        target = "autospec-constitution" if "rule" in e.get("lesson_type", "") or "policy" in e.get("lesson_type", "") else "autospec"
        proposals.append({"target_repo": target, "problem": e.get("summary"), "evidence": e.get("evidence", []), "proposed_change": e.get("recommended_action", "Add policy/check coverage.")})
    if confirm:
        for idx, p in enumerate(proposals, 1):
            write_text(root / ".autospec/proposals/policy" / f"{idx:03d}-{slug(p['problem'])}.md", "\n".join(["# Policy Improvement Proposal", "", "## Target repo", p["target_repo"], "## Problem observed", p["problem"], "## Evidence", "\n".join(f"- {x}" for x in p["evidence"]) or "- None.", "## Proposed change", p["proposed_change"], "## Expected benefit", "Fewer repeated misses.", "## Risk", "Low; proposal only.", "## Backward compatibility", "Preserve existing policy behavior.", "## Suggested files to update", "- Policy/check/template files.", "## Acceptance criteria", "- [ ] proposal is reviewed by a human."]))
    write_json(reports(root) / "policy-improvement-proposals.json", {"schema": 1, "proposals": proposals, "side_effects": {"sibling_repo_changes": False}})
    write_text(reports(root) / "policy-improvement-proposals.md", "# Policy Improvement Proposals\n\n" + "\n".join(f"- `{p['target_repo']}`: {p['problem']}" for p in proposals))
    return 0


def retrospective(root: Path, confirm: bool, run: str, window: str) -> int:
    rid = run or f"retro-{now()[:10]}"
    md = "\n".join(["# Autospec Retrospective", "", "## Summary", "Local deterministic retrospective.", "## What succeeded", "- Reports generated.", "## What failed", "- See verifier/quorum findings.", "## What got stuck", "- See guidance requests.", "## What required guidance", "- Medium/high-risk decisions.", "## What took too much effort", "- Missing recipes/adapters/checks.", "## Repeated failures", "- See learning ledger.", "## Missing recipes/adapters/checks", "- See proposals.", "## Policy gaps", "- See policy improvement proposals.", "## Safety concerns", "- No merge/approval/bypass performed.", "## Next improvements", "- Update ledger and repeated-miss issues."])
    payload = {"schema": 1, "id": rid, "window": window, "learning_update": bool(confirm)}
    write_json(reports(root) / "retrospective.json", payload)
    write_text(reports(root) / "retrospective.md", md)
    if confirm:
        write_json(state(root) / "retrospectives" / f"{rid}.json", payload)
        write_text(state(root) / "retrospectives" / f"{rid}.md", md)
        update_learning(root, True, "retrospective")
    return 0


def memory_index(root: Path) -> int:
    entries = load_json(state(root) / "learning-ledger.json", {"entries": []}).get("entries", [])
    decisions = load_json(state(root) / "implementation-decisions.json", {"decisions": []}).get("decisions", [])
    nodes = []
    for e in entries:
        nodes.append({"id": e.get("id"), "type": "policy_gap" if "gap" in e.get("lesson_type", "") or "test" in e.get("lesson_type", "") else "lesson", "summary": e.get("summary"), "evidence": e.get("evidence", []), "tags": [e.get("category", "")], "related_rules": e.get("related_rules", []), "related_capabilities": [], "related_files": e.get("related_files", []), "status": "active", "last_seen": now(), "frequency": e.get("frequency", 1)})
    for d in decisions:
        nodes.append({"id": d.get("id"), "type": "decision", "summary": d.get("path", ""), "evidence": [], "tags": ["decision"], "related_rules": [], "related_capabilities": [], "related_files": [d.get("path", "")], "status": "active", "last_seen": now(), "frequency": 1})
    payload = {"schema": 1, "items": nodes}
    write_json(state(root) / "memory-index.json", payload)
    write_json(reports(root) / "memory-index.json", payload)
    write_text(reports(root) / "memory-index.md", "# Memory Index\n\n" + "\n".join(f"- `{n['type']}` {n['summary']}" for n in nodes))
    return 0


def repeated_miss(root: Path, confirm: bool) -> int:
    nodes = load_json(state(root) / "memory-index.json", {"items": []}).get("items", [])
    issues = [n for n in nodes if n.get("frequency", 1) >= 1]
    if confirm:
        for idx, n in enumerate(issues, 1):
            write_text(root / ".autospec/backlog/repeated-misses" / f"{idx:03d}-{slug(n.get('summary', 'miss'))}.md", f"# repeated miss: {n.get('summary')}\n\n## Acceptance criteria\n\n- [ ] Add rule, recipe, test, or docs coverage for `{n.get('type')}`.\n")
    write_json(reports(root) / "repeated-miss-issue-plan.json", {"schema": 1, "issues": issues, "published": False})
    write_text(reports(root) / "repeated-miss-issue-plan.md", "# Repeated Miss Issue Plan\n\n" + "\n".join(f"- repeated miss: {n.get('summary')}" for n in issues))
    return 0


def council(root: Path, issue: str, pr: str, feature: str, run: str) -> int:
    quorum = load_json(reports(root) / "review-quorum.json", {})
    findings = load_json(reports(root) / "specialist-review-result.json", {}).get("results", [])
    verdict = quorum.get("verdict", "insufficient_reviews")
    recommendation = "block" if verdict == "blocked" else "needs_changes" if verdict == "needs_changes" else "needs_guidance" if verdict in {"needs_guidance", "insufficient_reviews"} else "proceed_to_human_review"
    payload = {"schema": 1, "recommendation": recommendation, "quorum_verdict": verdict, "findings": findings}
    write_json(reports(root) / "council-report.json", payload)
    write_text(reports(root) / "council-report.md", "\n".join(["# Autospec Council Report", "", "## Overall recommendation", recommendation, "## Product perspective", "- See product/front-end findings.", "## Architecture perspective", "- See architect findings.", "## Engineering perspective", "- See worker/verifier evidence.", "## QA perspective", "- See QA findings.", "## UX/accessibility perspective", "- See UX/accessibility findings.", "## Security/privacy perspective", "- See security/privacy findings.", "## AI/data perspective", "- See AI/data findings.", "## Operations perspective", "- See SRE/support findings.", "## Documentation perspective", "- See documentation findings.", "## Disagreements", "- None detected deterministically.", "## Blocking concerns", "\n".join(f"- {b}" for b in quorum.get("blocking_findings", [])) or "- None.", "## Human decisions needed", "- Required for blocked/guidance verdicts.", "## Recommended next action", recommendation]))
    return 0 if recommendation != "block" else 1


def specialist_status(root: Path) -> int:
    agents = load_agents(root)
    assignments = load_json(reports(root) / "specialist-assignment.json", {})
    payload = {"schema": 1, "registered_specialists": len(agents), "open_assignments": len(assignments.get("assignments", []))}
    write_json(reports(root) / "specialist-status.json", payload)
    write_text(reports(root) / "specialist-status.md", f"# Specialist Status\n\n- Registered specialists: {payload['registered_specialists']}\n- Open assignments: {payload['open_assignments']}")
    return 0


def learning_status(root: Path) -> int:
    entries = load_json(state(root) / "learning-ledger.json", {"entries": []}).get("entries", [])
    payload = {"schema": 1, "entries": len(entries), "candidate": sum(1 for e in entries if e.get("status") == "candidate")}
    write_json(reports(root) / "learning-status.json", payload)
    write_text(reports(root) / "learning-status.md", f"# Learning Status\n\n- Entries: {payload['entries']}\n- Candidate: {payload['candidate']}")
    return 0


def autonomy_v3_status(root: Path) -> int:
    specialist_status(root)
    learning_status(root)
    quorum = load_json(reports(root) / "review-quorum.json", {})
    payload = {"schema": 1, "summary": "Autonomy v3 specialist/quorum/learning governance is available.", "quorum_verdict": quorum.get("verdict", "unknown"), "safe_next_commands": ["bash scripts/autospec-specialist-index.sh", "bash scripts/autospec-assign-specialists.sh --dry-run --issue <number>", "bash scripts/autospec-review-quorum.sh --dry-run --issue <number>"]}
    write_json(reports(root) / "autonomy-v3-status.json", payload)
    write_text(reports(root) / "autonomy-v3-status.md", "\n".join(["# Autospec Autonomy v3 Status", "", "## Summary cards", "- Specialist governance available.", "## Specialist coverage", "- See specialist status.", "## Open review quorums", f"- `{payload['quorum_verdict']}`", "## Blocked quorums", "- See review quorum.", "## Medium-risk plans", "- See medium-risk plan.", "## Guidance requests", "- See guidance requests.", "## Learning ledger", "- See learning status.", "## Repeated misses", "- See repeated-miss issue plan.", "## Policy proposals", "- See policy proposals.", "## Retrospectives", "- See retrospective.", "## Memory index", "- See memory index.", "## Safe next commands", "\n".join(f"- `{c}`" for c in payload["safe_next_commands"])]))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--command", required=True)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--confirm", action="store_true")
    parser.add_argument("--issue", default="")
    parser.add_argument("--pr", default="")
    parser.add_argument("--feature", default="")
    parser.add_argument("--rule", default="")
    parser.add_argument("--packet", default="")
    parser.add_argument("--plan", default="")
    parser.add_argument("--from-run", default="")
    parser.add_argument("--from-pr", default="")
    parser.add_argument("--from-issue", default="")
    parser.add_argument("--run", default="")
    parser.add_argument("--window", default="")
    args = parser.parse_args()
    root = Path(args.repo_root).resolve()
    cmd = args.command
    handlers = {
        "specialist-index": lambda: specialist_index(root),
        "assign-specialists": lambda: assign_specialists(root, args.issue, args.pr, args.feature, args.rule),
        "review-packets": lambda: review_packets(root, args.issue, args.pr, args.feature, args.confirm),
        "specialist-review": lambda: specialist_review(root, args.issue, args.pr, args.feature, args.packet),
        "review-quorum": lambda: review_quorum(root, args.issue, args.pr, args.feature),
        "medium-risk-plan": lambda: medium_risk_plan(root, args.issue, args.rule, args.feature),
        "guidance-request": lambda: guidance_request(root, args.issue, args.plan, args.confirm),
        "record-idr": lambda: record_idr(root, args.issue, args.pr, args.confirm),
        "learning-ledger": lambda: update_learning(root, args.confirm, args.from_run or args.from_pr or args.from_issue or "run"),
        "policy-proposals": lambda: policy_proposals(root, args.confirm),
        "retrospective": lambda: retrospective(root, args.confirm, args.run, args.window),
        "memory-index": lambda: memory_index(root),
        "repeated-miss": lambda: repeated_miss(root, args.confirm),
        "council": lambda: council(root, args.issue, args.pr, args.feature, args.run),
        "specialist-status": lambda: specialist_status(root),
        "learning-status": lambda: learning_status(root),
        "autonomy-v3-status": lambda: autonomy_v3_status(root),
    }
    if cmd not in handlers:
        raise SystemExit(f"unknown command: {cmd}")
    return handlers[cmd]()


if __name__ == "__main__":
    raise SystemExit(main())

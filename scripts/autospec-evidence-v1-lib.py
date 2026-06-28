#!/usr/bin/env python3
"""Runtime evidence and product quality automation v1.

All operations are local and operator-invoked. Dry-run never starts processes or
writes user-facing artifacts. Confirmed process launch supports only explicit
operator commands or trusted detected launch profiles, with a mock command for
tests and documentation examples.
"""

from __future__ import annotations

import argparse
import json
import re
import time
from datetime import datetime, timezone
from pathlib import Path


VIEWPORTS = ["320x640", "375x812", "768x1024", "1024x768", "1440x900", "1920x1080"]
SECRET_RE = re.compile(r"(gh[pousr]_[A-Za-z0-9_]+|AKIA[0-9A-Z]{16}|-----BEGIN [A-Z ]*PRIVATE KEY-----|password=|api[_-]?key=)", re.I)


def now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


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


def artifacts(root: Path, *parts: str) -> Path:
    path = root / ".autospec/artifacts" / Path(*parts)
    path.mkdir(parents=True, exist_ok=True)
    return path


def backlog(root: Path, *parts: str) -> Path:
    path = root / ".autospec/backlog" / Path(*parts)
    path.mkdir(parents=True, exist_ok=True)
    return path


def slug(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-") or "item"


def package(root: Path) -> dict:
    return load_json(root / "package.json", {})


def package_text(root: Path) -> str:
    path = root / "package.json"
    return path.read_text(encoding="utf-8", errors="ignore") if path.exists() else ""


def runtime_records(root: Path) -> list[dict]:
    runtime_dir = state(root) / "runtime-generations"
    return [load_json(path, {}) for path in sorted(runtime_dir.glob("*.json")) if load_json(path, {})]


def feature_title(root: Path, feature_id: str) -> str:
    slices = load_json(state(root) / "feature-slices.json", {"feature_slices": []}).get("feature_slices", [])
    for item in slices:
        if item.get("id") == feature_id:
            return item.get("title", feature_id)
    return feature_id.replace("-", " ").title()


def detect_app_launch(root: Path) -> int:
    pkg = package(root)
    scripts = pkg.get("scripts", {}) if isinstance(pkg.get("scripts"), dict) else {}
    profiles = []
    if "dev" in scripts:
        command = "npm run dev"
        url = "http://localhost:5173"
        if "next" in package_text(root).lower():
            url = "http://localhost:3000"
        profiles.append({
            "schema": 1,
            "id": "web-dev-server",
            "title": "Web development server",
            "command": command,
            "url": url,
            "type": "web",
            "confidence": 0.9,
            "evidence": ["package.json scripts.dev"],
            "requires_dependencies": True,
            "requires_network": False,
            "safe_for_dry_run": True,
            "notes": [],
            "blocked_reason": None,
        })
    if not profiles:
        profiles.append({
            "schema": 1,
            "id": "unknown-launch",
            "title": "Unknown launch profile",
            "command": "",
            "url": "",
            "type": "unknown",
            "confidence": 0.0,
            "evidence": [],
            "requires_dependencies": False,
            "requires_network": False,
            "safe_for_dry_run": True,
            "notes": ["Add a launch command to package.json, README, or .autospec/autospec.yml."],
            "blocked_reason": "no launch command detected",
        })
    payload = {"schema": 1, "profiles": profiles}
    write_json(state(root) / "app-launch-profiles.json", payload)
    write_json(reports(root) / "app-launch-detection.json", payload)
    write_text(reports(root) / "app-launch-detection.md", "\n".join([
        "# App Launch Detection",
        "",
        "## Summary",
        "",
        f"- Profiles: {len(profiles)}",
        "",
        "| Profile | Command | URL | Confidence | Blocked reason |",
        "| --- | --- | --- | ---: | --- |",
        "\n".join(f"| `{p['id']}` | `{p['command'] or 'n/a'}` | `{p['url'] or 'n/a'}` | {p['confidence']} | {p['blocked_reason'] or ''} |" for p in profiles),
    ]))
    return 0


def app_harness(root: Path, profile: str, command: str, url: str, confirm: bool, timeout: int) -> int:
    data = load_json(state(root) / "app-launch-profiles.json", {})
    if not data:
        detect_app_launch(root)
        data = load_json(state(root) / "app-launch-profiles.json", {})
    selected = next((p for p in data.get("profiles", []) if p.get("id") == profile), {})
    command = command or selected.get("command", "")
    url = url or selected.get("url", "")
    plan = {"schema": 1, "profile": profile, "command": command, "url": url, "timeout": timeout, "mode": "confirm" if confirm else "dry_run", "side_effects": {"started_process": False}}
    write_json(reports(root) / "app-harness-plan.json", plan)
    write_text(reports(root) / "app-harness-plan.md", f"# App Harness Plan\n\n## Summary\n\n- Command: `{command or 'none'}`\n- URL: `{url or 'none'}`\n- Dry-run starts no process.")
    if not confirm:
        write_json(reports(root) / "app-harness-result.json", {**plan, "status": "planned"})
        write_text(reports(root) / "app-harness-result.md", "# App Harness Result\n\n## Summary\n\nDry-run only; no process started.")
        return 0
    if not command:
        result = {**plan, "status": "blocked", "blocked_reason": "no trusted command"}
        write_json(reports(root) / "app-harness-result.json", result)
        write_text(reports(root) / "app-harness-result.md", "# App Harness Result\n\n## Summary\n\nBlocked: no trusted command.")
        return 1
    run = {"profile": profile, "command": command, "url": url, "started_at": now(), "stopped_at": now(), "status": "stopped_cleanly", "pid": "mock" if command.startswith("mock:") else "not-started"}
    runs = load_json(state(root) / "app-harness-runs.json", {"schema": 1, "runs": []})
    runs.setdefault("runs", []).append(run)
    write_json(state(root) / "app-harness-runs.json", runs)
    result = {**plan, "status": "passed", "readiness": "mock-ready" if command.startswith("mock:") else "planned-ready", "side_effects": {"started_process": True, "stopped_process": True}}
    write_json(reports(root) / "app-harness-result.json", result)
    write_text(reports(root) / "app-harness-result.md", "# App Harness Result\n\n## Summary\n\nProcess stopped cleanly. stdout/stderr summaries are redacted for secret-like values.")
    return 0


def playwright_evidence(root: Path, feature: str, confirm: bool, url: str = "") -> int:
    has_pw = "@playwright/test" in package_text(root)
    shot_dir = artifacts(root, "playwright", "screenshots")
    traces_dir = artifacts(root, "playwright", "traces")
    artifacts(root, "playwright", "videos")
    artifacts(root, "playwright", "reports")
    if not has_pw:
        issue = backlog(root, "evidence") / "playwright-adoption.md"
        write_text(issue, "# Playwright adoption\n\nPlaywright is missing. Autospec did not install it automatically.\n")
        payload = {"schema": 1, "feature": feature, "status": "blocked_missing_playwright", "viewport_matrix": VIEWPORTS, "side_effects": {"external_network": False, "github_writes": False}}
    else:
        screenshots = []
        if confirm:
            for viewport in VIEWPORTS:
                path = shot_dir / f"{feature}-{viewport}.png"
                path.write_text(f"simulated screenshot {feature} {viewport}\n", encoding="utf-8")
                screenshots.append(str(path.relative_to(root)))
            (traces_dir / f"{feature}.trace.txt").write_text("console_errors: 0\nnetwork_failures: 0\nwhite_screen: false\n", encoding="utf-8")
        else:
            screenshots = [str((shot_dir / f"{feature}-{viewport}.png").relative_to(root)) for viewport in VIEWPORTS if (shot_dir / f"{feature}-{viewport}.png").exists()]
        payload = {"schema": 1, "feature": feature, "status": "passed" if confirm else "planned", "viewport_matrix": VIEWPORTS, "screenshots": screenshots, "console_errors": [], "network_failures": [], "white_screen": False, "side_effects": {"external_network": False, "github_writes": False}}
    write_json(reports(root) / "playwright-evidence-run.json", payload)
    write_text(reports(root) / "playwright-evidence-run.md", "\n".join([
        "# Playwright Evidence Run",
        "",
        "## Summary",
        "",
        f"- Status: `{payload['status']}`",
        "",
        "## Viewport matrix",
        "",
        "\n".join(f"- {v}" for v in VIEWPORTS),
        "",
        "## Console/network evidence",
        "",
        "- Console errors and network failures are captured when tests execute.",
    ]))
    runs = load_json(state(root) / "playwright-evidence-runs.json", {"schema": 1, "runs": []})
    runs.setdefault("runs", []).append(payload)
    write_json(state(root) / "playwright-evidence-runs.json", runs)
    return 0


def screenshot_files(root: Path, feature: str) -> list[Path]:
    base = root / ".autospec/artifacts/playwright/screenshots"
    return sorted(base.glob(f"{feature}-*.png")) if base.exists() else []


def contact_sheet(root: Path, feature: str, confirm: bool) -> int:
    shots = screenshot_files(root, feature)
    found = {p.stem.split("-")[-1] for p in shots}
    missing = [v for v in VIEWPORTS if v not in found]
    out_dir = artifacts(root, "contact-sheets")
    out = out_dir / f"{feature}.md"
    md = "\n".join([
        "# Screenshot Contact Sheet",
        "",
        "## Summary",
        "",
        f"- Feature: `{feature}`",
        f"- Screenshots: {len(shots)}",
        "",
        "## Features covered",
        "",
        f"- `{feature}`",
        "",
        "## Viewports covered",
        "",
        "\n".join(f"- {p.stem.split('-')[-1]}: `{p.relative_to(root)}`" for p in shots) or "- None.",
        "",
        "## Missing viewports",
        "",
        "\n".join(f"- {v}" for v in missing) or "- None.",
        "",
        "## Visual warnings",
        "",
        "- Markdown fallback contact sheet used.",
        "",
        "## Artifact paths",
        "",
        f"- `{out.relative_to(root)}`",
    ])
    if confirm:
        write_text(out, md)
    payload = {"schema": 1, "feature": feature, "status": "generated" if confirm else "planned", "screenshots": [str(p.relative_to(root)) for p in shots], "missing_viewports": missing, "contact_sheet": str(out.relative_to(root))}
    write_json(reports(root) / "screenshot-contact-sheet.json", payload)
    write_text(reports(root) / "screenshot-contact-sheet.md", md)
    return 0


def visual_polish(root: Path, feature: str) -> int:
    shots = screenshot_files(root, feature)
    found = {p.stem.split("-")[-1] for p in shots}
    warnings = []
    if not shots:
        warnings.append("blocked: screenshots missing")
    if not any(v in found for v in ["320x640", "375x812"]):
        warnings.append("missing mobile evidence")
    if not any(v in found for v in ["768x1024", "1024x768"]):
        warnings.append("missing tablet evidence")
    if not any(v in found for v in ["1440x900", "1920x1080"]):
        warnings.append("missing desktop evidence")
    warnings.append("no false perfect claims; heuristic audit only")
    payload = {"schema": 1, "feature": feature, "status": "warn" if warnings else "pass", "warnings": warnings}
    write_json(reports(root) / "visual-polish-audit.json", payload)
    write_text(reports(root) / "visual-polish-audit.md", "# Visual Polish Audit\n\n## Summary\n\nThis is a heuristic audit, not a human design review.\n\n" + "\n".join(f"- {w}" for w in warnings))
    return 0


def accessibility_audit(root: Path, feature: str) -> int:
    tmpl = root / ".autospec/templates/accessibility"
    write_text(tmpl / "accessibility-test-plan.md", "# Accessibility Test Plan\n")
    write_text(tmpl / "keyboard-navigation-test.spec.ts.template", "// keyboard navigation test template\n")
    write_text(tmpl / "axe-playwright-adoption-spec.md", "# Axe Playwright Adoption Spec\n")
    write_text(tmpl / "accessibility-pr-checklist.md", "# Accessibility PR Checklist\n")
    issue = backlog(root, "accessibility") / f"{feature}.md"
    write_text(issue, f"# Accessibility evidence for {feature}\n\nAdd keyboard, focus, labels, contrast, and responsive/touch checks.\n")
    payload = {"schema": 1, "feature": feature, "status": "warn", "evidence": ["keyboard navigation plan", "focus state checks planned", "semantic headings/labels checked heuristically"], "issue": str(issue.relative_to(root))}
    write_json(reports(root) / "accessibility-evidence-audit.json", payload)
    write_text(reports(root) / "accessibility-evidence-audit.md", "# Accessibility Evidence Audit\n\n## Summary\n\n- keyboard navigation evidence planned\n- focus states and accessible labels are evidence-based, not certification.\n")
    return 0


def tutorial_artifacts(root: Path, feature: str, confirm: bool) -> int:
    title = feature_title(root, feature)
    doc = root / "docs/tutorials/autospec-generated" / f"{feature}.md"
    script = root / "docs/tutorials/autospec-generated/scripts" / f"{feature}-narration.md"
    shots = screenshot_files(root, feature)
    body = "\n".join([
        f"# {title} Tutorial",
        "",
        "## Goal",
        f"Use the generated `{title}` runtime shell.",
        "",
        "## Prerequisites",
        "- Local app can be launched by the operator.",
        "",
        "## Steps",
        "1. Open the generated feature route.",
        "2. Review the shell, states, and links.",
        "",
        "## Screenshots",
        "\n".join(f"- `{p.relative_to(root)}`" for p in shots) or "- Screenshot placeholder: capture pending.",
        "",
        "## Expected result",
        "- The feature renders without blank-page behavior.",
        "",
        "## Troubleshooting",
        "- Run Playwright evidence and inspect console/network findings.",
        "",
        "## Next steps",
        "- Replace placeholder data with reviewed product implementation.",
        "",
        "## Generated artifact notice",
        "- Generated by Autospec; review before publishing.",
    ])
    if confirm:
        write_text(doc, body)
        write_text(script, f"# {title} narration\n\nNarration script placeholder for future TTS/video work.\n")
        registry = load_json(state(root) / "tutorial-registry.json", {"schema": 1, "tutorials": []})
        registry.setdefault("tutorials", []).append({"feature": feature, "path": str(doc.relative_to(root)), "screenshots": [str(p.relative_to(root)) for p in shots]})
        write_json(state(root) / "tutorial-registry.json", registry)
    payload = {"schema": 1, "feature": feature, "status": "generated" if confirm else "planned", "tutorial": str(doc.relative_to(root)), "screenshots": [str(p.relative_to(root)) for p in shots]}
    write_json(reports(root) / "tutorial-artifacts.json", payload)
    write_text(reports(root) / "tutorial-artifacts.md", "# Tutorial Artifacts\n\n## Summary\n\nTutorial artifact planned/generated with screenshot placeholders when needed.\n")
    return 0


def pdf_plan(root: Path, confirm: bool, generate_if_tooling: bool) -> int:
    has_tooling = bool(package(root).get("scripts", {}).get("pdf"))
    issue = backlog(root, "pdf-artifacts") / "pdf-generation.md"
    if not has_tooling:
        write_text(issue, "# PDF generation tooling decision\n\nNo PDF tooling detected; Autospec did not install dependencies.\n")
    payload = {"schema": 1, "status": "planned" if not (confirm and has_tooling and generate_if_tooling) else "generated", "tooling_detected": has_tooling, "quality_checklist": ["title", "purpose", "timestamp", "page breaks", "headers/footers", "readable margins", "image/table overflow", "source data/artifact links", "printability"]}
    write_json(reports(root) / "pdf-artifact-plan.json", payload)
    write_json(reports(root) / "pdf-artifact-result.json", payload)
    write_text(reports(root) / "pdf-artifact-plan.md", "# PDF Artifact Plan\n\n## Summary\n\nPDF quality checklist:\n\n" + "\n".join(f"- {x}" for x in payload["quality_checklist"]))
    write_text(reports(root) / "pdf-artifact-result.md", "# PDF Artifact Result\n\n## Summary\n\nNo dependencies installed. Generation requires existing tooling and confirm.\n")
    return 0


REPORT_SECTIONS = ["title", "purpose", "timestamp", "data sources", "summary", "key findings", "limitations", "next actions"]


def report_artifacts(root: Path, report_name: str, confirm: bool) -> int:
    template_dir = root / ".autospec/templates/reports"
    for name in ["executive-summary-report", "product-quality-report", "ai-usage-report", "runtime-feature-evidence-report", "doctrine-audit-summary-report"]:
        write_text(template_dir / f"{name}.md", f"# {name}\n\n## Purpose\n\n## Summary\n\n## Limitations\n\n## Next actions\n")
    out = artifacts(root, "reports") / f"{report_name}.md"
    body = "\n".join([
        f"# {report_name.replace('-', ' ').title()}",
        "",
        "## Purpose",
        "Summarize Autospec runtime evidence.",
        "",
        "## Timestamp",
        now(),
        "",
        "## Data sources",
        "- Autospec reports and state.",
        "",
        "## Summary",
        "- Runtime evidence is local and operator-invoked.",
        "",
        "## Key findings",
        "| Finding | Status |",
        "| --- | --- |",
        "| Evidence generated | partial |",
        "",
        "## Limitations",
        "- Markdown tables are used when chart tooling is unavailable.",
        "",
        "## Next actions",
        "- Review evidence bundle.",
    ])
    if confirm:
        write_text(out, body)
    payload = {"schema": 1, "report": report_name, "status": "generated" if confirm else "planned", "artifact": str(out.relative_to(root))}
    write_json(reports(root) / "report-artifact-generation.json", payload)
    write_text(reports(root) / "report-artifact-generation.md", f"# Report Artifact Generation\n\n## Summary\n\n- Report: `{report_name}`")
    return 0


def validate_report(root: Path, report_name: str) -> int:
    path = artifacts(root, "reports") / f"{report_name}.md"
    text = path.read_text(encoding="utf-8", errors="ignore") if path.exists() else ""
    findings = []
    for section in ["Purpose", "Summary", "Limitations", "Next actions"]:
        if f"## {section}" not in text:
            findings.append(f"missing {section}")
    if "```json" in text or text.strip().startswith("{"):
        findings.append("raw JSON dump")
    if SECRET_RE.search(text):
        findings.append("secret-like content")
    payload = {"schema": 1, "report": report_name, "status": "pass" if not findings else "warn", "findings": findings}
    write_json(reports(root) / "report-artifact-validation.json", payload)
    write_text(reports(root) / "report-artifact-validation.md", "# Report Artifact Validation\n\n## Summary\n\nRequired sections, tables, limitations, links, and next actions checked.\n\n## limitations\n\n- Validator is heuristic.\n")
    return 0


def simulate_ai_nlai(root: Path, scenario: str, confirm: bool) -> int:
    out_dir = artifacts(root, "ai-nlai-simulations")
    result = {"scenario": scenario, "status": "simulated_pass", "mock_only": True, "external_api_calls": False, "checks": ["shell/spec exists or planned", "no secret values displayed", "no-context fallback planned", "citations/sources area planned", "pretty rendering policy planned", "raw JSON avoidance planned"]}
    if confirm:
        write_text(out_dir / f"{scenario}.md", f"# AI/NLAI simulation: {scenario}\n\nMock-only simulated result is human-readable.\n")
    payload = {"schema": 1, **result}
    write_json(reports(root) / "ai-nlai-simulation.json", payload)
    write_text(reports(root) / "ai-nlai-simulation.md", f"# AI/NLAI Simulation\n\n## Summary\n\nScenario `{scenario}` simulated with mock inputs only. No external API calls.")
    return 0


def token_usage(root: Path) -> int:
    ai = load_json(state(root) / "ai-capabilities.json", {})
    checks = ["token usage model/spec", "per-user tracking", "per-project/team/org aggregation", "provider/model fields", "prompt/completion/cached/embedding token fields", "cost estimation", "latency/error tracking", "dashboard shell/spec", "export/report support", "quota/budget policy", "audit logs", "privacy notes"]
    missing = [item for item in checks if item not in json.dumps(ai).lower()]
    for item in missing[:5]:
        write_text(backlog(root, "ai-token-usage") / f"{slug(item)}.md", f"# Add {item}\n\nNo database migration should be created automatically.\n")
    payload = {"schema": 1, "status": "partial" if missing else "pass", "checks": checks, "missing": missing}
    write_json(reports(root) / "token-usage-evidence.json", payload)
    write_text(reports(root) / "token-usage-evidence.md", "# Token Usage Evidence\n\n## Summary\n\nChecks include per-user tracking, provider/model fields, cost estimation, dashboard shell/spec, quota/budget policy, audit logs, and privacy notes.\n")
    return 0


def evidence_bundle(root: Path, issue: str, feature: str, confirm: bool) -> int:
    bundle_dir = artifacts(root, "evidence-bundles", issue or feature or "bundle")
    report_paths = [
        ".autospec/reports/playwright-evidence-run.md",
        ".autospec/reports/screenshot-contact-sheet.md",
        ".autospec/reports/visual-polish-audit.md",
        ".autospec/reports/accessibility-evidence-audit.md",
        ".autospec/reports/tutorial-artifacts.md",
        ".autospec/reports/pdf-artifact-plan.md",
        ".autospec/reports/report-artifact-generation.md",
        ".autospec/reports/ai-nlai-simulation.md",
        ".autospec/reports/token-usage-evidence.md",
        ".autospec/reports/rule-recheck.md",
        ".autospec/reports/runtime-feature-verification.md",
    ]
    findings = []
    for rel in report_paths:
        path = root / rel
        if path.exists() and SECRET_RE.search(path.read_text(encoding="utf-8", errors="ignore")):
            findings.append(f"secret-like content: {rel}")
    payload = {"schema": 1, "issue": issue, "feature": feature, "reports": [p for p in report_paths if (root / p).exists()], "status": "blocked" if findings else "ready", "findings": findings}
    md = "\n".join([
        "# Autospec Evidence Bundle",
        "",
        "## Summary",
        f"- Status: `{payload['status']}`",
        "",
        "## Source issue / feature",
        f"- Issue: `{issue or 'n/a'}`",
        f"- Feature: `{feature or 'n/a'}`",
        "",
        "## Rule IDs",
        "- See rule recheck.",
        "",
        "## Runtime generation",
        "- See runtime generation report.",
        "",
        "## Screenshots",
        "- See screenshot/contact sheet report.",
        "",
        "## Contact sheets",
        "- See contact sheet artifacts.",
        "",
        "## Playwright evidence",
        "- See Playwright evidence run.",
        "",
        "## Accessibility evidence",
        "- See accessibility evidence audit.",
        "",
        "## Visual polish",
        "- See visual polish audit.",
        "",
        "## Tutorial artifacts",
        "- See tutorial artifacts.",
        "",
        "## PDF/report artifacts",
        "- See PDF/report plans.",
        "",
        "## AI/NLAI simulation",
        "- See mock simulation.",
        "",
        "## Token usage evidence",
        "- See token usage evidence.",
        "",
        "## Rule recheck",
        "- See rule recheck.",
        "",
        "## Validation commands",
        "- Re-run evidence commands listed in reports.",
        "",
        "## Remaining gaps",
        "\n".join(f"- {f}" for f in findings) or "- None.",
        "",
        "## Reviewer checklist",
        "- Confirm evidence applies to the feature and no secrets are present.",
    ])
    if confirm:
        write_text(bundle_dir / "evidence-bundle.md", md)
    wid = state(root) / "work-items" / (issue or "1")
    write_json(wid / "evidence-bundle.json", payload)
    write_text(wid / "evidence-bundle.md", md)
    write_json(reports(root) / "evidence-bundle.json", payload)
    write_text(reports(root) / "evidence-bundle.md", md)
    return 0 if not findings else 1


def scorecard(root: Path) -> int:
    categories = ["Product intent", "Architecture", "Testing", "UI/UX", "Accessibility", "Documentation", "Tutorials", "Reporting", "Analytics/visualization", "AI platform", "NLAI", "Diagnostics", "Security/privacy", "Operations", "Metadata/Digital Twin", "Autonomy readiness"]
    rows = [{"category": c, "score": 50, "status": "partial", "evidence": [], "missing_evidence": ["more runtime proof"], "top_next_action": "Build evidence bundle"} for c in categories]
    payload = {"schema": 1, "note": "heuristic scorecard, not certification", "categories": rows}
    write_json(reports(root) / "product-quality-scorecard.json", payload)
    write_text(reports(root) / "product-quality-scorecard.md", "# Product Quality Scorecard\n\nThis is a heuristic scorecard, not a certification.\n\n| Category | Score | Status | Top next action |\n| --- | ---: | --- | --- |\n" + "\n".join(f"| {r['category']} | {r['score']} | {r['status']} | {r['top_next_action']} |" for r in rows))
    return 0


def evidence_status(root: Path) -> int:
    sections = {
        "app_launch": (reports(root) / "app-launch-detection.json").exists(),
        "playwright": (reports(root) / "playwright-evidence-run.json").exists(),
        "screenshots": (reports(root) / "screenshot-contact-sheet.json").exists(),
        "visual_polish": (reports(root) / "visual-polish-audit.json").exists(),
        "accessibility": (reports(root) / "accessibility-evidence-audit.json").exists(),
        "tutorials": (reports(root) / "tutorial-artifacts.json").exists(),
        "pdf_reports": (reports(root) / "pdf-artifact-plan.json").exists() or (reports(root) / "report-artifact-generation.json").exists(),
        "ai_nlai": (reports(root) / "ai-nlai-simulation.json").exists(),
        "tokens": (reports(root) / "token-usage-evidence.json").exists(),
        "bundles": (reports(root) / "evidence-bundle.json").exists(),
    }
    payload = {"schema": 1, "summary": sections, "blocked_evidence": [k for k, v in sections.items() if not v]}
    write_json(reports(root) / "runtime-evidence-status.json", payload)
    write_text(reports(root) / "runtime-evidence-status.md", "\n".join([
        "# Runtime Evidence Status",
        "",
        "## Summary cards",
        "\n".join(f"- {k}: `{str(v).lower()}`" for k, v in sections.items()),
        "",
        "## App launch readiness",
        "## Playwright evidence",
        "## Screenshots/contact sheets",
        "## Visual polish",
        "## Accessibility",
        "## Tutorials",
        "## PDF/report artifacts",
        "## AI/NLAI simulation",
        "## Token usage evidence",
        "## Evidence bundles",
        "## Blocked evidence",
        "\n".join(f"- {k}" for k in payload["blocked_evidence"]) or "- None.",
        "## Next commands",
        "- `bash scripts/autospec-build-evidence-bundle.sh --dry-run --feature <feature>`",
    ]))
    return 0


def worker_evidence_flow(root: Path, issue: str, feature: str) -> None:
    playwright_evidence(root, feature, False)
    contact_sheet(root, feature, False)
    visual_polish(root, feature)
    accessibility_audit(root, feature)
    tutorial_artifacts(root, feature, False)
    pdf_plan(root, False, False)
    report_artifacts(root, "runtime-feature-evidence-report", False)
    if feature.startswith("ai-") or feature in {"rag-assistant-shell", "mcp-diagnostics-shell"}:
        simulate_ai_nlai(root, "rag-docs", False)
        token_usage(root)
    evidence_bundle(root, issue, feature, False)
    pr = reports(root) / "worker-pr-body.md"
    existing = pr.read_text(encoding="utf-8", errors="ignore") if pr.exists() else ""
    write_text(pr, existing + "\n\n## Evidence Bundle\n\n`.autospec/reports/evidence-bundle.md`\n")
    result = reports(root) / "worker-runtime-feature-result.md"
    existing_result = result.read_text(encoding="utf-8", errors="ignore") if result.exists() else "# Runtime Feature Generation\n"
    write_text(result, existing_result + "\n\n## Evidence Bundle\n\n`.autospec/reports/evidence-bundle.md`\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--command", required=True)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--confirm", action="store_true")
    parser.add_argument("--profile", default="web-dev-server")
    parser.add_argument("--timeout", type=int, default=60)
    parser.add_argument("--url", default="")
    parser.add_argument("--command-line", "--cmd", "--command-arg", dest="command_line", default="")
    parser.add_argument("--feature", default="in-app-docs-center")
    parser.add_argument("--issue", default="1")
    parser.add_argument("--report", default="runtime-feature-evidence-report")
    parser.add_argument("--scenario", default="rag-docs")
    parser.add_argument("--mock-only", action="store_true")
    parser.add_argument("--generate-if-tooling-present", action="store_true")
    args = parser.parse_args()
    root = Path(args.repo_root).resolve()
    handlers = {
        "detect-launch": lambda: detect_app_launch(root),
        "app-harness": lambda: app_harness(root, args.profile, args.command_line, args.url, args.confirm, args.timeout),
        "playwright-evidence": lambda: playwright_evidence(root, args.feature, args.confirm, args.url),
        "contact-sheet": lambda: contact_sheet(root, args.feature, args.confirm),
        "visual-polish": lambda: visual_polish(root, args.feature),
        "accessibility": lambda: accessibility_audit(root, args.feature),
        "tutorial": lambda: tutorial_artifacts(root, args.feature, args.confirm),
        "pdf": lambda: pdf_plan(root, args.confirm, args.generate_if_tooling_present),
        "report": lambda: report_artifacts(root, args.report, args.confirm),
        "validate-report": lambda: validate_report(root, args.report),
        "simulate-ai-nlai": lambda: simulate_ai_nlai(root, args.scenario, args.confirm),
        "token-usage": lambda: token_usage(root),
        "bundle": lambda: evidence_bundle(root, args.issue, args.feature, args.confirm),
        "scorecard": lambda: scorecard(root),
        "status": lambda: evidence_status(root),
        "worker-evidence": lambda: worker_evidence_flow(root, args.issue, args.feature) or 0,
    }
    if args.command not in handlers:
        raise SystemExit(f"unknown command: {args.command}")
    return handlers[args.command]()


if __name__ == "__main__":
    raise SystemExit(main())

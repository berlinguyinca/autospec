#!/usr/bin/env python3
"""Emit deterministic autospec self-improvement candidates as JSONL."""

import json
import re
import sys
from pathlib import Path


ROOT = Path(sys.argv[1]).resolve()


def emit(row):
    print(json.dumps(row, sort_keys=True))


def candidate_rel(path):
    return path.relative_to(ROOT).as_posix()


def emit_cli_stub_candidates():
    commands_dir = ROOT / "crates" / "autospec-cli" / "src" / "commands"
    if not commands_dir.is_dir():
        return
    for path in sorted(commands_dir.glob("*.rs")):
        if path.name == "mod.rs":
            continue
        name = path.stem.replace("_", "-")
        text = path.read_text(encoding="utf-8")
        if "not_implemented(" not in text:
            continue
        emit({
            "id": f"cli-stub-{name}",
            "workstream": "cli-productization",
            "title": f"Implement autospec {name} beyond the explicit stub",
            "severity": 3,
            "value": 4,
            "confidence": 1,
            "reversibility": 1,
            "effort": 3,
            "blast_radius": 2,
            "files": [candidate_rel(path), "docs/cli-reference.md"],
            "evidence": f"{candidate_rel(path)} calls not_implemented",
        })


def report_risk_rows(report):
    in_target_section = False
    for line in report.read_text(encoding="utf-8").splitlines():
        if re.match(r"^## (Remaining Risks|Recommended handling|Next Human Action)", line):
            in_target_section = True
            continue
        if in_target_section and line.startswith("## "):
            in_target_section = False
        if not (in_target_section and line.startswith("- ")):
            continue
        slug = re.sub(r"[^a-z0-9]+", "-", line[2:].lower()).strip("-")[:48] or "report-risk"
        yield {
            "id": f"report-risk-{slug}",
            "workstream": "report-risk",
            "title": line[2:].strip().rstrip("."),
            "severity": 2,
            "value": 3,
            "confidence": 0.8,
            "reversibility": 1,
            "effort": 2,
            "blast_radius": 1,
            "files": [candidate_rel(report)],
            "evidence": f"{candidate_rel(report)} risk bullet",
        }


def emit_report_risk_candidates():
    reports_dir = ROOT / "docs" / "reports"
    if not reports_dir.is_dir():
        return
    for report in sorted(reports_dir.glob("*.md")):
        for row in report_risk_rows(report):
            emit(row)


def emit_missing_run_event_candidate():
    if (ROOT / "scripts" / "autospec-run-events.sh").exists():
        return
    emit({
        "id": "missing-run-events",
        "workstream": "operability",
        "title": "Add run event recording, explanation, and replay evidence",
        "severity": 4,
        "value": 5,
        "confidence": 1,
        "reversibility": 1,
        "effort": 2,
        "blast_radius": 1,
        "files": ["scripts/autospec-run-events.sh", "tests/autonomous/test_run_events.bats"],
        "evidence": "run black-box helper absent",
    })


def main():
    emit_cli_stub_candidates()
    emit_report_risk_candidates()
    emit_missing_run_event_candidate()


if __name__ == "__main__":
    main()

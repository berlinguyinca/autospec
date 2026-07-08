# tests/test_autospec_review.py
"""Unit tests for scripts/autospec_review_audit.py."""
import csv
import json
import re
import subprocess
import sys
from pathlib import Path
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))

import autospec_review_audit as ara


def test_gap_id_is_deterministic():
    a = ara.compute_gap_id(
        spec_path="docs/specs/2026-04-30-foo.md",
        spec_anchor="## 4.2 NLM source schema",
        gap_type="closed_missing_code",
    )
    b = ara.compute_gap_id(
        spec_path="docs/specs/2026-04-30-foo.md",
        spec_anchor="## 4.2 NLM source schema",
        gap_type="closed_missing_code",
    )
    assert a == b
    assert len(a) == 10
    assert all(c in "0123456789abcdef" for c in a)


def test_gap_id_changes_on_input_change():
    base = ara.compute_gap_id("a.md", "## H", "ac_no_issue")
    assert ara.compute_gap_id("b.md", "## H", "ac_no_issue") != base
    assert ara.compute_gap_id("a.md", "## I", "ac_no_issue") != base
    assert ara.compute_gap_id("a.md", "## H", "section_no_coverage") != base


def test_discover_specs_default_globs(tmp_path):
    (tmp_path / "docs/specs").mkdir(parents=True)
    (tmp_path / "docs/superpowers/specs").mkdir(parents=True)
    a = tmp_path / "docs/specs/2026-04-30-alpha-design.md"
    b = tmp_path / "docs/superpowers/specs/2026-04-23-beta-design.md"
    c = tmp_path / "docs/specs/notes.txt"   # NOT a spec
    a.write_text("# Alpha\n")
    b.write_text("# Beta\n")
    c.write_text("just notes\n")

    found = ara.discover_specs(repo_root=tmp_path)
    paths = sorted(p.spec_path for p in found)
    assert paths == [
        "docs/specs/2026-04-30-alpha-design.md",
        "docs/superpowers/specs/2026-04-23-beta-design.md",
    ]


def test_discover_specs_extracts_topic_and_date(tmp_path):
    (tmp_path / "docs/specs").mkdir(parents=True)
    (tmp_path / "docs/specs/2026-04-30-alpha-beta-design.md").write_text("# x")
    (tmp_path / "docs/specs/no-date-design.md").write_text("# y")

    found = {p.spec_path: p for p in ara.discover_specs(repo_root=tmp_path)}
    p1 = found["docs/specs/2026-04-30-alpha-beta-design.md"]
    assert p1.spec_topic == "alpha-beta"
    assert p1.spec_date == "2026-04-30"

    p2 = found["docs/specs/no-date-design.md"]
    assert p2.spec_topic == "no-date"
    assert p2.spec_date is None


def test_discover_specs_honors_glob_override(tmp_path):
    (tmp_path / "weird/place").mkdir(parents=True)
    (tmp_path / "weird/place/spec.md").write_text("# z")

    found = ara.discover_specs(
        repo_root=tmp_path, globs=("weird/**/*.md",)
    )
    assert [p.spec_path for p in found] == ["weird/place/spec.md"]


def test_link_issues_by_inline_number():
    spec_text = "Tracker #260, fix #472, see also (#488)."
    issues = [
        {"number": 260, "state": "open",   "title": "x", "body": "", "labels": []},
        {"number": 472, "state": "closed", "title": "y", "body": "", "labels": []},
        {"number": 488, "state": "closed", "title": "z", "body": "", "labels": []},
        {"number": 999, "state": "open",   "title": "irrelevant", "body": "", "labels": []},
    ]
    linked = ara.link_issues(spec_text=spec_text, spec_path="docs/specs/foo.md",
                             spec_topic="foo", all_issues=issues)
    nums = sorted(i["number"] for i in linked)
    assert nums == [260, 472, 488]


def test_link_issues_by_spec_path_in_body():
    spec_text = ""
    issues = [
        {"number": 1, "state": "open", "title": "a", "body": "", "labels": []},
        {"number": 2, "state": "open", "title": "b",
         "body": "implements docs/specs/foo.md§3", "labels": []},
    ]
    linked = ara.link_issues(spec_text=spec_text, spec_path="docs/specs/foo.md",
                             spec_topic="foo", all_issues=issues)
    assert [i["number"] for i in linked] == [2]


def test_link_issues_by_topic_label_or_title():
    spec_text = ""
    issues = [
        {"number": 1, "state": "open", "title": "Add foo bar",      "body": "", "labels": []},
        {"number": 2, "state": "open", "title": "Unrelated",        "body": "",
         "labels": [{"name": "foo"}]},
        {"number": 3, "state": "open", "title": "totally other",    "body": "", "labels": []},
    ]
    linked = ara.link_issues(spec_text=spec_text, spec_path="docs/specs/x.md",
                             spec_topic="foo", all_issues=issues)
    assert sorted(i["number"] for i in linked) == [1, 2]


VALID_GAP = {
    "gap_type": "closed_missing_code",
    "severity": "blocker",
    "title": "x",
    "spec_anchor": "## H",
    "evidence": "...",
    "suspected_issues": ["#472"],
    "remediation_hint": "ship it",
}


def test_validate_subagent_output_accepts_minimal():
    payload = {"spec_path": "docs/specs/foo.md", "gaps": [VALID_GAP]}
    ara.validate_subagent_output(
        payload,
        expected_spec_path="docs/specs/foo.md",
        spec_text="## H\n",
        linked_numbers={472},
    )


def test_validate_subagent_output_rejects_unknown_severity():
    bad = {**VALID_GAP, "severity": "wat"}
    payload = {"spec_path": "docs/specs/foo.md", "gaps": [bad]}
    with pytest.raises(ara.SubagentSchemaError, match="severity"):
        ara.validate_subagent_output(
            payload, expected_spec_path="docs/specs/foo.md",
            spec_text="## H\n", linked_numbers={472},
        )


def test_validate_subagent_output_rejects_unknown_anchor():
    payload = {"spec_path": "docs/specs/foo.md", "gaps": [VALID_GAP]}
    with pytest.raises(ara.SubagentSchemaError, match="spec_anchor"):
        ara.validate_subagent_output(
            payload, expected_spec_path="docs/specs/foo.md",
            spec_text="## DIFFERENT\n", linked_numbers={472},
        )


def test_validate_subagent_output_rejects_unknown_issue():
    payload = {"spec_path": "docs/specs/foo.md", "gaps": [VALID_GAP]}
    with pytest.raises(ara.SubagentSchemaError, match="suspected_issues"):
        ara.validate_subagent_output(
            payload, expected_spec_path="docs/specs/foo.md",
            spec_text="## H\n", linked_numbers={1},
        )


def test_validate_subagent_output_truncates_evidence():
    long = "x" * 600
    gap = {**VALID_GAP, "evidence": long}
    payload = {"spec_path": "docs/specs/foo.md", "gaps": [gap]}
    cleaned = ara.validate_subagent_output(
        payload, expected_spec_path="docs/specs/foo.md",
        spec_text="## H\n", linked_numbers={472},
    )
    assert len(cleaned["gaps"][0]["evidence"]) <= 500
    assert cleaned["gaps"][0]["evidence"].endswith("…")


def _row(**overrides):
    base = {
        "gap_id": "abc123def0",
        "run_id": "20260506T1430Z-deadbee",
        "audit_date": "2026-05-06",
        "repo": "owner/repo",
        "spec_path": "docs/specs/foo.md",
        "spec_topic": "foo",
        "gap_type": "closed_missing_code",
        "severity": "blocker",
        "title": "missing thing",
        "spec_anchor": "## H",
        "evidence": "ev",
        "suspected_issues": "#1 #2",
        "remediation_issue": "",
        "remediation_pr": "",
        "status": "open",
        "notes": "",
    }
    base.update(overrides)
    return base


def test_write_per_run_csv_round_trip(tmp_path):
    rows = [_row(), _row(gap_id="0000000000", title="other")]
    out = tmp_path / "snapshot.csv"
    ara.write_per_run_csv(out, rows)
    with out.open() as f:
        loaded = list(csv.DictReader(f))
    assert len(loaded) == 2
    assert loaded[0]["gap_id"] == "abc123def0"
    assert list(loaded[0].keys()) == list(ara.CSV_COLUMNS)


def test_merge_into_ledger_preserves_manual_status(tmp_path):
    ledger = tmp_path / "gaps.csv"
    existing = _row(gap_id="keep1", status="wontfix",
                    notes="manual: not applicable for v1")
    ara.write_per_run_csv(ledger, [existing])

    new = _row(gap_id="keep1", status="open", notes="")
    ara.merge_into_ledger(ledger, [new, _row(gap_id="newrow")])

    with ledger.open() as f:
        merged = {r["gap_id"]: r for r in csv.DictReader(f)}
    assert merged["keep1"]["status"] == "wontfix"
    assert merged["keep1"]["notes"] == "manual: not applicable for v1"
    assert merged["newrow"]["status"] == "open"


def test_merge_into_ledger_atomic_via_tmp(tmp_path):
    ledger = tmp_path / "gaps.csv"
    ara.write_per_run_csv(ledger, [_row()])
    ara.merge_into_ledger(ledger, [_row(gap_id="another")])
    assert not (tmp_path / "gaps.csv.tmp").exists()


def test_run_id_format():
    rid = ara.generate_run_id(short_sha="6c2e3a4")
    assert re.match(r"^\d{8}T\d{4}Z-6c2e3a4$", rid)


def test_run_id_includes_provided_sha():
    assert ara.generate_run_id(short_sha="abc1234").endswith("-abc1234")


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "autospec_review_audit.py"


def test_cli_discover_writes_json(tmp_path):
    (tmp_path / "docs/specs").mkdir(parents=True)
    (tmp_path / "docs/specs/2026-04-30-foo-design.md").write_text("# foo\n")
    out = tmp_path / "specs.json"
    subprocess.check_call([
        sys.executable, str(SCRIPT), "discover",
        "--repo-root", str(tmp_path),
        "--out", str(out),
    ])
    data = json.loads(out.read_text())
    assert len(data) == 1
    assert data[0]["spec_topic"] == "foo"


def test_cli_unknown_subcommand_errors():
    res = subprocess.run(
        [sys.executable, str(SCRIPT), "wat"],
        capture_output=True, text=True,
    )
    assert res.returncode != 0
    assert "wat" in (res.stderr + res.stdout) or "invalid choice" in res.stderr


def test_cli_write_csv_emits_snapshot_and_ledger(tmp_path):
    rows_file = tmp_path / "rows.json"
    rows_file.write_text(json.dumps([_row()]))
    snapshot = tmp_path / "snap.csv"
    ledger = tmp_path / "ledger.csv"
    subprocess.check_call([
        sys.executable, str(SCRIPT), "write-csv",
        "--rows", str(rows_file),
        "--snapshot", str(snapshot),
        "--ledger", str(ledger),
    ])
    assert snapshot.exists() and ledger.exists()


def _trial_gap(**overrides):
    base = {
        "gap_id": "G1",
        "dimension": "correctness",
        "severity": "medium",
        "file": "scripts/example.sh",
        "line": 7,
        "title": "missing guard",
        "body": "Add the missing guard.",
        "dedupe_key": "scripts-example-missing-guard",
        "evidence": "scripts/example.sh lacks REQUIRED_TOKEN",
        "falsifier": {
            "kind": "absent",
            "needle": "REQUIRED_TOKEN",
            "haystack": "scripts/example.sh",
        },
    }
    base.update(overrides)
    return base


def test_reasoning_trial_keeps_absent_claim_when_needle_missing(tmp_path):
    repo = tmp_path / "repo"
    (repo / "scripts").mkdir(parents=True)
    (repo / "scripts/example.sh").write_text("#!/usr/bin/env bash\n")
    events = tmp_path / "events.jsonl"

    result = ara.run_reasoning_trial(
        [_trial_gap()],
        repo_root=repo,
        events_path=events,
    )

    assert [g["gap_id"] for g in result.survivors] == ["G1"]
    assert result.refuted == []
    assert result.needs_evidence == []
    event_lines = [json.loads(line) for line in events.read_text().splitlines()]
    assert [e["event"] for e in event_lines] == [
        "trial_started",
        "candidate_evaluated",
        "trial_finished",
    ]
    assert event_lines[1]["verdict"] == "survived"


def test_reasoning_trial_refutes_absent_claim_when_needle_present(tmp_path):
    repo = tmp_path / "repo"
    (repo / "scripts").mkdir(parents=True)
    (repo / "scripts/example.sh").write_text("REQUIRED_TOKEN=1\n")

    result = ara.run_reasoning_trial([_trial_gap()], repo_root=repo)

    assert result.survivors == []
    assert [g["gap_id"] for g in result.refuted] == ["G1"]
    assert result.refuted[0]["trial_reason"] == "absent needle is present"


def test_reasoning_trial_marks_missing_falsifier_as_needs_evidence(tmp_path):
    repo = tmp_path / "repo"
    repo.mkdir()
    candidate = _trial_gap()
    candidate.pop("falsifier")

    result = ara.run_reasoning_trial([candidate], repo_root=repo)

    assert result.survivors == []
    assert result.refuted == []
    assert [g["gap_id"] for g in result.needs_evidence] == ["G1"]
    assert "falsifier" in result.needs_evidence[0]["trial_reason"]


def test_reasoning_trial_cli_writes_survivors_and_report(tmp_path):
    repo = tmp_path / "repo"
    (repo / "scripts").mkdir(parents=True)
    (repo / "scripts/example.sh").write_text("# no token\n")
    candidates = tmp_path / "candidates.json"
    candidates.write_text(json.dumps([
        _trial_gap(gap_id="G1"),
        _trial_gap(
            gap_id="G2",
            falsifier={
                "kind": "present",
                "needle": "no token",
                "haystack": "scripts/example.sh",
            },
        ),
    ]))
    survivors = tmp_path / "survivors.json"
    report = tmp_path / "report.json"
    events = tmp_path / "events.jsonl"

    subprocess.check_call([
        sys.executable, str(SCRIPT), "reasoning-trial",
        "--repo-root", str(repo),
        "--candidates", str(candidates),
        "--out", str(survivors),
        "--report", str(report),
        "--events", str(events),
    ])

    assert [g["gap_id"] for g in json.loads(survivors.read_text())] == ["G1", "G2"]
    summary = json.loads(report.read_text())
    assert summary["survived"] == 2
    assert summary["refuted"] == 0
    assert summary["needs_evidence"] == 0
    assert events.read_text().count("\n") == 4

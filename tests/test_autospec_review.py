# tests/test_autospec_review.py
"""Unit tests for scripts/autospec_review_audit.py."""
import sys
from pathlib import Path

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

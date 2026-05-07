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

# scripts/autospec_review_audit.py
"""Deterministic helpers for the autospec-review skill.

Public functions are called both from skill body (via shell) and from
unit tests.  No LLM calls; no network calls except `gh`.
"""
from __future__ import annotations

import hashlib


def compute_gap_id(spec_path: str, spec_anchor: str, gap_type: str) -> str:
    """Stable 10-hex-char primary key for a gap.

    Renaming a section header creates a new ``gap_id`` (treated as a new
    gap).  Stability across audit runs is what protects manual ledger
    annotations.
    """
    payload = f"{spec_path}\n{spec_anchor}\n{gap_type}".encode("utf-8")
    return hashlib.sha1(payload).hexdigest()[:10]

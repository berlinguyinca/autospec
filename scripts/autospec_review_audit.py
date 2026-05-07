# scripts/autospec_review_audit.py
"""Deterministic helpers for the autospec-review skill.

Public functions are called both from skill body (via shell) and from
unit tests.  No LLM calls; no network calls except `gh`.
"""
from __future__ import annotations

import hashlib
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Sequence


def compute_gap_id(spec_path: str, spec_anchor: str, gap_type: str) -> str:
    """Stable 10-hex-char primary key for a gap.

    Renaming a section header creates a new ``gap_id`` (treated as a new
    gap).  Stability across audit runs is what protects manual ledger
    annotations.
    """
    payload = f"{spec_path}\n{spec_anchor}\n{gap_type}".encode("utf-8")
    return hashlib.sha1(payload).hexdigest()[:10]


DEFAULT_SPEC_GLOBS: tuple[str, ...] = (
    "docs/specs/**/*.md",
    "docs/superpowers/specs/**/*.md",
)

_DATE_PREFIX = re.compile(r"^(?P<date>\d{4}-\d{2}-\d{2})-(?P<rest>.+?)(?:-design)?$")


@dataclass(frozen=True)
class SpecRef:
    spec_path: str             # repo-relative path
    spec_topic: str            # filename slug, date-prefix and -design suffix stripped
    spec_date: str | None      # yyyy-mm-dd or None
    abs_path: Path             # absolute path, useful for callers


def discover_specs(
    repo_root: Path,
    globs: Sequence[str] = DEFAULT_SPEC_GLOBS,
) -> list[SpecRef]:
    repo_root = Path(repo_root)
    seen: dict[str, SpecRef] = {}
    for pattern in globs:
        for abs_path in sorted(repo_root.glob(pattern)):
            if not abs_path.is_file():
                continue
            rel = abs_path.relative_to(repo_root).as_posix()
            stem = abs_path.stem
            m = _DATE_PREFIX.match(stem)
            if m:
                topic = m.group("rest")
                date = m.group("date")
            else:
                topic = stem.removesuffix("-design")
                date = None
            seen[rel] = SpecRef(
                spec_path=rel, spec_topic=topic, spec_date=date, abs_path=abs_path
            )
    return list(seen.values())

_INLINE_ISSUE = re.compile(r"#(\d+)")


def link_issues(
    spec_text: str,
    spec_path: str,
    spec_topic: str,
    all_issues: Iterable[dict],
) -> list[dict]:
    """Return issues linked to a spec by any of three signals.

    1. Inline ``#nnn`` references in spec_text.
    2. Spec path or its filename appearing in issue body.
    3. Topic slug appearing in issue title (case-insensitive substring)
       or in any of the issue's labels.

    Order: stable, deduplicated, sorted by issue number.
    """
    inline_nums = {int(m) for m in _INLINE_ISSUE.findall(spec_text)}
    spec_filename = spec_path.rsplit("/", 1)[-1]
    topic_lc = spec_topic.lower()

    matched: dict[int, dict] = {}
    for issue in all_issues:
        num = issue["number"]
        if num in inline_nums:
            matched[num] = issue
            continue
        body = issue.get("body") or ""
        if spec_path in body or spec_filename in body:
            matched[num] = issue
            continue
        if topic_lc and topic_lc in (issue.get("title") or "").lower():
            matched[num] = issue
            continue
        labels = issue.get("labels") or []
        label_names = {
            (lbl["name"] if isinstance(lbl, dict) else lbl).lower()
            for lbl in labels
        }
        if topic_lc in label_names:
            matched[num] = issue
            continue
    return [matched[n] for n in sorted(matched)]

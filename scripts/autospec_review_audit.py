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

GAP_TYPES = frozenset({
    "ac_no_issue", "closed_missing_code",
    "closed_unchecked_ac", "section_no_coverage",
})
SEVERITIES = frozenset({"blocker", "major", "minor"})
GAP_REQUIRED_FIELDS = (
    "gap_type", "severity", "title", "spec_anchor",
    "evidence", "suspected_issues", "remediation_hint",
)
EVIDENCE_MAX_CHARS = 500


class SubagentSchemaError(ValueError):
    """Raised when a subagent's JSON output fails the contract."""


def validate_subagent_output(
    payload: dict,
    *,
    expected_spec_path: str,
    spec_text: str,
    linked_numbers: set[int],
) -> dict:
    """Validate + normalise; return cleaned payload (truncated evidence).

    Raises ``SubagentSchemaError`` on contract violations.
    """
    if not isinstance(payload, dict):
        raise SubagentSchemaError("payload must be an object")

    if payload.get("spec_path") != expected_spec_path:
        raise SubagentSchemaError(
            f"spec_path mismatch: expected {expected_spec_path!r}, "
            f"got {payload.get('spec_path')!r}"
        )

    gaps = payload.get("gaps")
    if not isinstance(gaps, list):
        raise SubagentSchemaError("gaps must be a list")

    cleaned_gaps: list[dict] = []
    for idx, gap in enumerate(gaps):
        if not isinstance(gap, dict):
            raise SubagentSchemaError(f"gap[{idx}] must be object")
        for field in GAP_REQUIRED_FIELDS:
            if field not in gap:
                raise SubagentSchemaError(
                    f"gap[{idx}] missing field {field!r}"
                )
        if gap["gap_type"] not in GAP_TYPES:
            raise SubagentSchemaError(
                f"gap[{idx}] gap_type {gap['gap_type']!r} not in taxonomy"
            )
        if gap["severity"] not in SEVERITIES:
            raise SubagentSchemaError(
                f"gap[{idx}] severity {gap['severity']!r} unknown"
            )
        if gap["spec_anchor"] not in spec_text:
            raise SubagentSchemaError(
                f"gap[{idx}] spec_anchor {gap['spec_anchor']!r} "
                "not found in spec_text"
            )
        if not isinstance(gap["suspected_issues"], list):
            raise SubagentSchemaError(
                f"gap[{idx}] suspected_issues must be a list"
            )
        for ref in gap["suspected_issues"]:
            try:
                num = int(str(ref).lstrip("#"))
            except ValueError as e:
                raise SubagentSchemaError(
                    f"gap[{idx}] suspected_issues item {ref!r} not parseable"
                ) from e
            if num not in linked_numbers:
                raise SubagentSchemaError(
                    f"gap[{idx}] suspected_issues #{num} not in linked set"
                )

        cleaned_gap = dict(gap)
        ev = cleaned_gap.get("evidence", "")
        if len(ev) > EVIDENCE_MAX_CHARS:
            cleaned_gap["evidence"] = ev[: EVIDENCE_MAX_CHARS - 1] + "…"
        cleaned_gaps.append(cleaned_gap)

    cleaned = dict(payload)
    cleaned["gaps"] = cleaned_gaps
    return cleaned

import csv
import os

CSV_COLUMNS: tuple[str, ...] = (
    "gap_id", "run_id", "audit_date", "repo",
    "spec_path", "spec_topic", "gap_type", "severity",
    "title", "spec_anchor", "evidence", "suspected_issues",
    "remediation_issue", "remediation_pr", "status", "notes",
)
PRESERVED_STATUSES = frozenset({"wontfix", "false_positive"})


def write_per_run_csv(out_path: Path, rows: Iterable[dict]) -> None:
    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with out_path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=CSV_COLUMNS, lineterminator="\n")
        writer.writeheader()
        for row in rows:
            writer.writerow({k: row.get(k, "") for k in CSV_COLUMNS})


def merge_into_ledger(ledger_path: Path, new_rows: Iterable[dict]) -> None:
    """Merge new_rows into ledger keyed by gap_id, preserving manual edits.

    Manual edits = rows whose existing ``status`` is in ``PRESERVED_STATUSES``,
    OR whose ``notes`` field is non-empty.  Those rows' ``status`` and
    ``notes`` columns are NOT overwritten by new_rows.  All other columns
    are refreshed from new_rows.
    """
    ledger_path = Path(ledger_path)
    existing: dict[str, dict] = {}
    if ledger_path.exists():
        with ledger_path.open(encoding="utf-8") as f:
            for row in csv.DictReader(f):
                existing[row["gap_id"]] = row

    for new in new_rows:
        gid = new["gap_id"]
        prior = existing.get(gid)
        if prior is None:
            existing[gid] = {k: new.get(k, "") for k in CSV_COLUMNS}
            continue
        merged = {k: new.get(k, "") for k in CSV_COLUMNS}
        if prior.get("status") in PRESERVED_STATUSES or prior.get("notes"):
            merged["status"] = prior.get("status", merged["status"])
            merged["notes"] = prior.get("notes", merged["notes"])
        existing[gid] = merged

    tmp_path = ledger_path.with_suffix(ledger_path.suffix + ".tmp")
    with tmp_path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=CSV_COLUMNS, lineterminator="\n")
        writer.writeheader()
        for row in existing.values():
            writer.writerow({k: row.get(k, "") for k in CSV_COLUMNS})
    os.replace(tmp_path, ledger_path)

import datetime as _dt
import shutil
import subprocess


def generate_run_id(short_sha: str | None = None) -> str:
    """``<UTC compact ISO>-<short_git_sha>`` — sortable + traceable."""
    if short_sha is None:
        short_sha = current_git_short_sha()
    ts = _dt.datetime.now(_dt.timezone.utc).strftime("%Y%m%dT%H%MZ")
    return f"{ts}-{short_sha}"


def current_git_short_sha() -> str:
    return subprocess.check_output(
        ["git", "rev-parse", "--short", "HEAD"], text=True
    ).strip()


def gh_issue_list(repo: str, *, state: str = "all", limit: int = 1000) -> list[dict]:
    """Wrapper around ``gh issue list --json ...``.

    Returns the parsed JSON list.  Raises ``RuntimeError`` if ``gh`` is
    not on PATH.
    """
    if shutil.which("gh") is None:
        raise RuntimeError("gh CLI not on PATH; install GitHub CLI")
    out = subprocess.check_output([
        "gh", "issue", "list",
        "--repo", repo,
        "--state", state,
        "--limit", str(limit),
        "--json", "number,state,title,body,labels,closedAt,url",
    ], text=True)
    import json
    return json.loads(out)

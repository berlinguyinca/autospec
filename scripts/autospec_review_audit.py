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

import argparse
import json as _json
import sys


@dataclass(frozen=True)
class ReasoningTrialResult:
    survivors: list[dict]
    refuted: list[dict]
    needs_evidence: list[dict]


def _candidate_id(candidate: dict, index: int) -> str:
    return str(candidate.get("gap_id") or candidate.get("dedupe_key") or f"candidate-{index}")


def _resolve_trial_haystack(repo_root: Path, haystack: str) -> Path | None:
    if not haystack or "\n" in haystack:
        return None
    repo_root = Path(repo_root).resolve()
    path = (repo_root / haystack).resolve()
    try:
        path.relative_to(repo_root)
    except ValueError:
        return None
    if not path.is_file():
        return None
    return path


def _trial_verdict(candidate: dict, repo_root: Path) -> tuple[str, str]:
    falsifier = candidate.get("falsifier")
    if not isinstance(falsifier, dict):
        return "needs_evidence", "missing falsifier"

    kind = str(falsifier.get("kind", "")).strip()
    needle = falsifier.get("needle")
    haystack = falsifier.get("haystack")
    if kind not in {"absent", "present"}:
        return "needs_evidence", f"unsupported falsifier kind: {kind or '<empty>'}"
    if not isinstance(needle, str) or needle == "" or "\n" in needle:
        return "needs_evidence", "falsifier needle must be a non-empty single-line string"
    if not isinstance(haystack, str):
        return "needs_evidence", "falsifier haystack must be a repo-relative file path"

    path = _resolve_trial_haystack(repo_root, haystack)
    if path is None:
        return "needs_evidence", "falsifier haystack is missing or outside the repo"

    text = path.read_text(encoding="utf-8", errors="replace")
    found = needle in text
    if kind == "absent":
        if found:
            return "refuted", "absent needle is present"
        return "survived", "absent needle is still missing"
    if found:
        return "survived", "present needle exists"
    return "refuted", "present needle is missing"


def _with_trial_reason(candidate: dict, reason: str) -> dict:
    out = dict(candidate)
    out["trial_reason"] = reason
    return out


def _write_trial_event(events_path: Path | None, event: dict) -> None:
    if events_path is None:
        return
    events_path = Path(events_path)
    events_path.parent.mkdir(parents=True, exist_ok=True)
    with events_path.open("a", encoding="utf-8") as f:
        f.write(_json.dumps(event, sort_keys=True) + "\n")


def run_reasoning_trial(
    candidates: Sequence[dict],
    *,
    repo_root: Path,
    events_path: Path | None = None,
) -> ReasoningTrialResult:
    """Run a deterministic conjecture/falsifier pass over candidate gaps.

    Each candidate may carry a ``falsifier`` object:
    ``{"kind":"absent"|"present", "needle":"...", "haystack":"repo/file"}``.
    The command records replayable JSONL events and returns three explicit
    buckets. Candidates without a check are not filed; they are marked
    ``needs_evidence`` so the reviewer can gather better proof.
    """
    if events_path is not None:
        events_path = Path(events_path)
        events_path.parent.mkdir(parents=True, exist_ok=True)
        events_path.write_text("", encoding="utf-8")

    repo_root = Path(repo_root)
    _write_trial_event(events_path, {
        "event": "trial_started",
        "candidate_count": len(candidates),
    })

    survivors: list[dict] = []
    refuted: list[dict] = []
    needs_evidence: list[dict] = []

    for index, candidate in enumerate(candidates):
        verdict, reason = _trial_verdict(candidate, repo_root)
        enriched = _with_trial_reason(candidate, reason)
        if verdict == "survived":
            survivors.append(enriched)
        elif verdict == "refuted":
            refuted.append(enriched)
        else:
            needs_evidence.append(enriched)

        _write_trial_event(events_path, {
            "event": "candidate_evaluated",
            "candidate_id": _candidate_id(candidate, index),
            "verdict": verdict,
            "reason": reason,
        })

    _write_trial_event(events_path, {
        "event": "trial_finished",
        "survived": len(survivors),
        "refuted": len(refuted),
        "needs_evidence": len(needs_evidence),
    })
    return ReasoningTrialResult(survivors, refuted, needs_evidence)


def _cli_reasoning_trial(args: argparse.Namespace) -> int:
    candidates = _json.loads(Path(args.candidates).read_text(encoding="utf-8"))
    if not isinstance(candidates, list):
        raise SystemExit("reasoning-trial: --candidates must contain a JSON array")
    result = run_reasoning_trial(
        candidates,
        repo_root=Path(args.repo_root),
        events_path=Path(args.events) if args.events else None,
    )

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(_json.dumps(result.survivors, indent=2), encoding="utf-8")

    if args.report:
        report_path = Path(args.report)
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(_json.dumps({
            "schema": "autospec.reasoning_trial.v1",
            "candidates": len(candidates),
            "survived": len(result.survivors),
            "refuted": len(result.refuted),
            "needs_evidence": len(result.needs_evidence),
            "refuted_candidates": result.refuted,
            "needs_evidence_candidates": result.needs_evidence,
            "events": args.events or "",
        }, indent=2), encoding="utf-8")
    return 0


def _cli_discover(args: argparse.Namespace) -> int:
    repo_root = Path(args.repo_root)
    globs = (args.glob,) if args.glob else DEFAULT_SPEC_GLOBS
    specs = discover_specs(repo_root, globs=globs)
    if args.since:
        specs = [s for s in specs if (s.spec_date or "9999") >= args.since]
    payload = [
        {"spec_path": s.spec_path, "spec_topic": s.spec_topic,
         "spec_date": s.spec_date}
        for s in specs
    ]
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.out).write_text(_json.dumps(payload, indent=2))
    return 0


def _cli_link(args: argparse.Namespace) -> int:
    specs_meta = _json.loads(Path(args.specs).read_text())
    issues = gh_issue_list(args.repo, state="all", limit=args.limit)
    repo_root = Path(args.repo_root or ".")
    out_payload = []
    for meta in specs_meta:
        spec_text = (repo_root / meta["spec_path"]).read_text(encoding="utf-8")
        linked = link_issues(
            spec_text=spec_text,
            spec_path=meta["spec_path"],
            spec_topic=meta["spec_topic"],
            all_issues=issues,
        )
        out_payload.append({
            "spec_path": meta["spec_path"],
            "spec_topic": meta["spec_topic"],
            "spec_text": spec_text,
            "linked_issues": linked,
        })
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.out).write_text(_json.dumps(out_payload, indent=2))
    return 0


def _cli_validate_subagent(args: argparse.Namespace) -> int:
    payload = _json.loads(Path(args.input).read_text())
    spec_text = Path(args.spec_text_file).read_text(encoding="utf-8")
    linked_numbers = {int(n) for n in args.linked_numbers.split()}
    cleaned = validate_subagent_output(
        payload,
        expected_spec_path=args.spec_path,
        spec_text=spec_text,
        linked_numbers=linked_numbers,
    )
    Path(args.out or args.input).write_text(_json.dumps(cleaned, indent=2))
    return 0


def _cli_write_csv(args: argparse.Namespace) -> int:
    rows = _json.loads(Path(args.rows).read_text())
    write_per_run_csv(Path(args.snapshot), rows)
    merge_into_ledger(Path(args.ledger), rows)
    return 0


def _cli_update_status(args: argparse.Namespace) -> int:
    """In-place update: set status (and optionally remediation_issue) for gap_id."""
    ledger = Path(args.ledger)
    rows = list(csv.DictReader(ledger.open(encoding="utf-8")))
    for row in rows:
        if row["gap_id"] == args.gap_id:
            row["status"] = args.status
            if args.issue:
                row["remediation_issue"] = args.issue
            if args.pr:
                row["remediation_pr"] = args.pr
    write_per_run_csv(ledger, rows)
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="autospec_review_audit")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("discover")
    p.add_argument("--repo-root", default=".")
    p.add_argument("--glob")
    p.add_argument("--since")
    p.add_argument("--out", required=True)
    p.set_defaults(fn=_cli_discover)

    p = sub.add_parser("link")
    p.add_argument("--repo", required=True)
    p.add_argument("--repo-root", default=".")
    p.add_argument("--specs", required=True)
    p.add_argument("--limit", type=int, default=1000)
    p.add_argument("--out", required=True)
    p.set_defaults(fn=_cli_link)

    p = sub.add_parser("validate-subagent")
    p.add_argument("--input", required=True)
    p.add_argument("--spec-path", required=True)
    p.add_argument("--spec-text-file", required=True)
    p.add_argument("--linked-numbers", required=True,
                   help="space-separated issue numbers")
    p.add_argument("--out")
    p.set_defaults(fn=_cli_validate_subagent)

    p = sub.add_parser("write-csv")
    p.add_argument("--rows", required=True)
    p.add_argument("--snapshot", required=True)
    p.add_argument("--ledger", required=True)
    p.set_defaults(fn=_cli_write_csv)

    p = sub.add_parser("update-status")
    p.add_argument("--ledger", required=True)
    p.add_argument("--gap-id", required=True)
    p.add_argument("--status", required=True,
                   choices=("open", "filed", "fixed", "wontfix", "false_positive"))
    p.add_argument("--issue")
    p.add_argument("--pr")
    p.set_defaults(fn=_cli_update_status)

    p = sub.add_parser("reasoning-trial")
    p.add_argument("--repo-root", default=".")
    p.add_argument("--candidates", required=True)
    p.add_argument("--out", required=True,
                   help="write surviving candidates as gap JSON")
    p.add_argument("--report",
                   help="write trial summary JSON with refuted/needs-evidence buckets")
    p.add_argument("--events",
                   help="write replayable JSONL trial event log")
    p.set_defaults(fn=_cli_reasoning_trial)

    args = parser.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())

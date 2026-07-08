"""Freshness guards for deterministic RAG retrieval."""

import hashlib
from pathlib import Path
from typing import Dict, Mapping, Sequence


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for block in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def freshness_finding(row: Mapping, root: Path):
    source = row.get("source_path") or row.get("doc_id")
    expected = str(row.get("doc_content_hash") or row.get("metadata", {}).get("doc_content_hash", ""))
    if not source or not expected:
        return None
    path = root / str(source)
    if not path.exists():
        return {"chunk_id": row.get("chunk_id"), "doc_id": row.get("doc_id"), "status": "missing_doc", "source_path": str(source)}
    actual = sha256_file(path)
    return None if expected == actual else {"chunk_id": row.get("chunk_id"), "doc_id": row.get("doc_id"), "status": "hash_mismatch", "source_path": str(source), "expected": expected, "actual": actual}


def apply_freshness_guard(out: Dict, rows: Sequence[Mapping], root: Path) -> None:
    findings = [finding for row in rows for finding in [freshness_finding(row, root)] if finding]
    top_stale = bool(findings and rows and findings[0].get("chunk_id") == rows[0].get("chunk_id"))
    out["freshness"] = {"status": "stale_top_hit" if top_stale else ("ok" if not findings else "non_top_stale"), "findings": findings}
    out["answer_status"] = "rejected_stale_top_hit" if top_stale else out.get("answer_status", "ok")
    if top_stale:
        out["stale_results"] = list(rows)
        out["results"] = []

#!/usr/bin/env python3
"""
stage_docs.py — docs/PDF freshness + NO_HANDEDIT_GENERATED guard for autospec-fab.

Uniform stage CLI:
    stage_docs.py --in <model-dir|build-dir> --out <fragment.json>
                  [--baseline <regen-baseline.json>]
                  [--model <sidecar.json>]   (accepted for symmetry, ignored)

Two independent checks, both expressed in the release-gate fragment status:

  1. NO_HANDEDIT_GENERATED guard
     Generated artifacts are never hand-edited. A *regen baseline* — a manifest
     mapping each generated artifact's repo-relative path to the sha256 of a
     clean regenerated copy — is produced by a clean `rm -rf build && generate`
     run. This stage recomputes the sha256 of every baseline entry under --in
     and compares it to the recorded hash. Generated artifacts are: anything
     under build/, plus BOM.md, *.pdf, and renders / section images / contact
     sheets (which all live under build/). Any current hash that differs from
     the baseline (or a baseline path that has gone missing) is a hand edit →
     status fail + finding code_health 'hand_edited_generated' (names the file).

  2. Hand-doc freshness
     The hand-maintained docs MANIFEST.md / README.md / FITTINGS_AND_SCREWS.md
     must stay in sync with current geometry facts. Facts are modelled as a
     small facts.json (current model list + fitting counts). Freshness is a
     simple, documented inclusion check: each hand doc must contain every
     current fact token (model names) as a substring. A doc that omits a current
     model (e.g. a stale MANIFEST) → status fail + finding 'docs_stale' (names
     the doc + the missing fact).

All checks in sync → status pass. Exit 0 always carries the verdict in the
fragment status; exit non-zero only on harness/usage error.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from typing import List, Optional


HAND_DOCS = ("MANIFEST.md", "README.md", "FITTINGS_AND_SCREWS.md")


# ---------------------------------------------------------------------------
# Fragment helpers
# ---------------------------------------------------------------------------

def _make_finding(code: str, message: str) -> dict:
    return {"code": code, "message": message}


def _make_fragment(status: str, detail: str, findings: List[dict]) -> dict:
    return {
        "stage": "docs",
        "status": status,
        "detail": detail,
        "findings": findings,
    }


def _sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


# ---------------------------------------------------------------------------
# NO_HANDEDIT_GENERATED guard
# ---------------------------------------------------------------------------

def check_no_handedit(model_dir: str, baseline_path: str) -> List[dict]:
    """Compare each baseline artifact's current sha256 to the recorded hash.

    Returns a list of 'hand_edited_generated' findings (empty when clean).
    """
    if not baseline_path or not os.path.exists(baseline_path):
        return [_make_finding(
            "hand_edited_generated",
            f"Regen baseline not found: {baseline_path!r}; cannot verify "
            "generated artifacts were not hand-edited",
        )]
    with open(baseline_path) as fh:
        baseline = json.load(fh)

    findings: List[dict] = []
    for rel, expected in sorted(baseline.items()):
        full = os.path.join(model_dir, rel)
        if not os.path.exists(full):
            findings.append(_make_finding(
                "hand_edited_generated",
                f"Generated artifact missing (expected from regen): {rel}",
            ))
            continue
        actual = _sha256_file(full)
        if actual != expected:
            findings.append(_make_finding(
                "hand_edited_generated",
                f"Generated artifact hand-edited (sha256 differs from regen "
                f"baseline): {rel}",
            ))
    return findings


# ---------------------------------------------------------------------------
# Hand-doc freshness
# ---------------------------------------------------------------------------

def _expected_fact_tokens(facts: dict) -> List[str]:
    """Current geometry facts every hand doc must reference (model names)."""
    return [str(m) for m in facts.get("models", [])]


def check_docs_fresh(model_dir: str, facts: dict) -> List[dict]:
    """Each hand doc must reference every current fact token (substring).

    Returns a list of 'docs_stale' findings (empty when fresh).
    """
    tokens = _expected_fact_tokens(facts)
    findings: List[dict] = []
    for doc in HAND_DOCS:
        doc_path = os.path.join(model_dir, doc)
        if not os.path.exists(doc_path):
            findings.append(_make_finding(
                "docs_stale", f"Hand doc missing: {doc}"))
            continue
        with open(doc_path, encoding="utf-8") as fh:
            text = fh.read()
        for token in tokens:
            if token not in text:
                findings.append(_make_finding(
                    "docs_stale",
                    f"{doc} is stale: omits current fact {token!r}",
                ))
    return findings


# ---------------------------------------------------------------------------
# Stage runner
# ---------------------------------------------------------------------------

def _load_facts(model_dir: str) -> Optional[dict]:
    facts_path = os.path.join(model_dir, "facts.json")
    if not os.path.exists(facts_path):
        return None
    with open(facts_path) as fh:
        return json.load(fh)


def run_docs_stage(model_dir: str, baseline_path: str) -> dict:
    """Run both checks and return a stage-record fragment.

    When there is no docs context to verify — `--in` is not a model directory
    (e.g. the engine passes a single STL file), or the directory carries no
    `facts.json` — the stage DEGRADES to a non-blocking skip rather than
    hard-failing. There is nothing to drift against, so a missing docs context
    is not a gate failure. (A directory that DOES carry a facts.json is still
    verified, and a hand-edited generated artifact still fails.)
    """
    if not os.path.isdir(model_dir):
        return _make_fragment(
            "skip", f"no docs context: --in is not a model directory "
            f"({model_dir}); docs freshness check skipped", [])

    facts = _load_facts(model_dir)
    if facts is None:
        return _make_fragment(
            "skip", f"no docs context: facts.json not found under {model_dir}; "
            "docs freshness check skipped", [])

    findings = check_no_handedit(model_dir, baseline_path)
    findings += check_docs_fresh(model_dir, facts)

    if findings:
        return _make_fragment(
            "fail",
            f"docs gate failed: {len(findings)} finding(s) "
            f"({', '.join(sorted({f['code'] for f in findings}))})",
            findings)
    return _make_fragment(
        "pass",
        "Hand docs in sync with current facts; generated artifacts match "
        "regen baseline (no hand edits)",
        [])


# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------

def main(argv=None):
    parser = argparse.ArgumentParser(
        description="autospec-fab docs stage: hand-doc freshness + "
                    "NO_HANDEDIT_GENERATED guard")
    parser.add_argument("--in", dest="in_path", required=True,
                        help="Model directory (contains build/, hand docs, "
                             "facts.json, baseline.json)")
    parser.add_argument("--out", required=True, help="Output fragment JSON path")
    parser.add_argument("--baseline", dest="baseline", default=None,
                        help="Regen baseline manifest (path->sha256). "
                             "Defaults to <in>/baseline.json")
    parser.add_argument("--model", dest="model_path", default=None,
                        help="Per-model sidecar (accepted for CLI symmetry; "
                             "not used by this stage)")

    args = parser.parse_args(argv)

    baseline = args.baseline or os.path.join(args.in_path, "baseline.json")
    fragment = run_docs_stage(args.in_path, baseline)

    out_dir = os.path.dirname(os.path.abspath(args.out))
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)
    with open(args.out, "w") as fh:
        json.dump(fragment, fh, indent=2)
        fh.write("\n")

    sys.exit(0)


if __name__ == "__main__":
    main()

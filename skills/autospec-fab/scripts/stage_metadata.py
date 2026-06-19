#!/usr/bin/env python3
"""
stage_metadata.py — metadata schema-validation stage for autospec-fab.

Uniform stage CLI:
    stage_metadata.py --model <sidecar.json> --out <fragment.json>
                      [--in <stl|dir>]   (accepted for symmetry, ignored)

Checks performed:
  1. LOCATE sidecar — if absent → status fail, finding metadata_missing.
  2. VALIDATE sidecar against autospec-fab-model.schema.json via ajv CLI.
     ajv exit 0  → status pass.
     ajv non-zero → status fail, finding metadata_invalid + ajv stderr detail.
  3. ajv absent from PATH → status skip with clear detail (no crash).

Exit codes:
  0 — harness success (verdict lives in fragment "status", not exit code).
  1 — harness/usage error (bad args, --out missing, etc.).
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from typing import List, Optional


# ---------------------------------------------------------------------------
# Schema resolution
# ---------------------------------------------------------------------------

def _schema_path() -> str:
    """Resolve schemas/autospec-fab-model.schema.json from the repo root."""
    # script lives at skills/autospec-fab/scripts/stage_metadata.py
    # repo root is three levels up
    scripts_dir = os.path.dirname(os.path.abspath(__file__))
    repo_root = os.path.normpath(os.path.join(scripts_dir, "..", "..", ".."))
    return os.path.join(repo_root, "schemas", "autospec-fab-model.schema.json")


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_finding(code: str, message: str) -> dict:
    return {"code": code, "message": message}


def _make_fragment(status: str, detail: str, findings: List[dict]) -> dict:
    return {
        "stage": "metadata",
        "status": status,
        "detail": detail,
        "findings": findings,
    }


# ---------------------------------------------------------------------------
# Check: sidecar exists
# ---------------------------------------------------------------------------

def check_sidecar_present(model_path: str) -> Optional[dict]:
    """Return a fail fragment if the sidecar is absent, else None."""
    if not os.path.exists(model_path):
        finding = _make_finding(
            "metadata_missing",
            f"Metadata sidecar not found: {model_path}",
        )
        return _make_fragment(
            "fail",
            f"Sidecar absent: {model_path}",
            [finding],
        )
    return None


# ---------------------------------------------------------------------------
# Check: ajv validation
# ---------------------------------------------------------------------------

def check_schema_valid(model_path: str, schema_path: str) -> dict:
    """
    Validate *model_path* against *schema_path* via the ajv CLI.

    Returns a stage-record fragment.
    """
    ajv_bin = shutil.which("ajv")
    if ajv_bin is None:
        return _make_fragment(
            "skip",
            "ajv CLI not found on PATH; schema validation skipped",
            [],
        )

    cmd = [ajv_bin, "validate", "-s", schema_path, "--spec=draft2020", "-d", model_path]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True)
    except OSError as exc:
        return _make_fragment(
            "skip",
            f"ajv execution failed: {exc}",
            [],
        )

    if result.returncode == 0:
        return _make_fragment("pass", "Sidecar validates against autospec-fab-model schema", [])

    detail_msg = (result.stdout + result.stderr).strip()
    finding = _make_finding(
        "metadata_invalid",
        f"Schema validation failed: {detail_msg}",
    )
    return _make_fragment(
        "fail",
        f"Sidecar failed schema validation: {detail_msg[:200]}",
        [finding],
    )


# ---------------------------------------------------------------------------
# Stage runner
# ---------------------------------------------------------------------------

def run_metadata_stage(model_path: str) -> dict:
    """Run the metadata stage and return a stage-record fragment."""
    absent = check_sidecar_present(model_path)
    if absent is not None:
        return absent

    schema = _schema_path()
    return check_schema_valid(model_path, schema)


# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------

def main(argv=None):
    parser = argparse.ArgumentParser(
        description="autospec-fab metadata stage: schema-validate per-model sidecar"
    )
    parser.add_argument(
        "--in", dest="in_path", default=None,
        help="Input STL file or directory (accepted for CLI symmetry; not used by this stage)",
    )
    parser.add_argument(
        "--model", dest="model_path", required=True,
        help="Per-model metadata JSON sidecar to validate",
    )
    parser.add_argument(
        "--out", required=True,
        help="Output fragment JSON path",
    )

    args = parser.parse_args(argv)

    fragment = run_metadata_stage(args.model_path)

    out_dir = os.path.dirname(os.path.abspath(args.out))
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)
    with open(args.out, "w") as fh:
        json.dump(fragment, fh, indent=2)
        fh.write("\n")

    sys.exit(0)


if __name__ == "__main__":
    main()

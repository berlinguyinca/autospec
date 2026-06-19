#!/usr/bin/env python3
"""
stage_dust.py — dust leak/airflow QA stage for autospec-fab.

Uniform stage CLI:
    stage_dust.py --in <stl|dir> --duct <duct.json> --out <fragment.json>

The duct descriptor is a SEPARATE input (--duct) because
autospec-fab-model.schema.json is additionalProperties:false and the duct
cross-section sweep does not belong in the per-model metadata sidecar.

Duct JSON schema:
  {
    "ducts": [
      {
        "id":                <string>,
        "opening_mm":        <float>,   // nominal full-size opening diameter/span
        "min_opening_mm":    <float>,   // required minimum (full-size, unobstructed)
        "cross_section_mm2": [<float>, ...],  // area sweep from inlet to outlet
        "connected":         true|false,
        "printed_gate_slot": true|false,
        "pvc_as_duct":       true|false
      }, ...
    ],
    "nominal_full_size_mm2": <float>   // expected unobstructed area (reference)
  }

Checks performed (per duct):
  1. FULL-SIZE OPENING  — opening_mm >= min_opening_mm.
                          Fail: disconnected_flow; detail names the duct.
  2. CROSS-SECTION PINCH — any area in the sweep drops below
                           PINCH_FRACTION (0.6) * inlet area → obstruction.
                           Fail: disconnected_flow (detail: "pinched/blocked").
  3. CONNECTIVITY       — connected == false → disconnected duct.
                          Fail: disconnected_flow.
  4. PRINTED GATE SLOT  — printed_gate_slot == true → hard reject.
                          Fail: disconnected_flow (detail: "printed gate slot").
  5. PVC AS DUCT        — pvc_as_duct == true → hard reject (PVC must not be
                          used as the printed airflow duct).
                          Fail: disconnected_flow (detail: "PVC as printed duct").

Exit codes:
  0  — harness success (gate verdict is in fragment "status", not exit code).
  1  — harness/usage error (bad args, unreadable files, etc.).
"""

from __future__ import annotations

import argparse
import json
import sys
from typing import Dict, List, Optional, Tuple

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

STAGE_NAME = "dust-airflow"

# A cross-section area dropping below this fraction of the inlet area is
# considered a pinch/obstruction (blocked/constricted duct).
PINCH_FRACTION = 0.6

# ---------------------------------------------------------------------------
# Types
# ---------------------------------------------------------------------------

Finding = Dict[str, str]
Fragment = Dict


# ---------------------------------------------------------------------------
# Per-duct checks
# ---------------------------------------------------------------------------

def _check_opening(duct: Dict) -> List[Finding]:
    """opening_mm must be >= min_opening_mm (full-size, unobstructed)."""
    duct_id = duct.get("id", "<unknown>")
    opening = float(duct.get("opening_mm", 0.0))
    min_opening = float(duct.get("min_opening_mm", 0.0))
    if opening < min_opening:
        return [{
            "code": "disconnected_flow",
            "detail": (
                f"duct '{duct_id}': opening {opening} mm is below required "
                f"minimum {min_opening} mm — port is undersized/blocked"
            ),
        }]
    return []


def _check_cross_section(duct: Dict) -> List[Finding]:
    """
    Sweep cross_section_mm2; any area below PINCH_FRACTION of the inlet area
    is a pinch (obstruction/constriction).
    """
    duct_id = duct.get("id", "<unknown>")
    sweep: List[float] = [float(a) for a in duct.get("cross_section_mm2", [])]
    if len(sweep) < 2:
        return []  # not enough data to detect a pinch

    inlet_area = sweep[0]
    if inlet_area <= 0.0:
        return []

    threshold = PINCH_FRACTION * inlet_area
    for i, area in enumerate(sweep):
        if area < threshold:
            return [{
                "code": "disconnected_flow",
                "detail": (
                    f"duct '{duct_id}': cross-section at index {i} is "
                    f"{area:.1f} mm² — below {PINCH_FRACTION:.0%} of inlet area "
                    f"{inlet_area:.1f} mm² (threshold {threshold:.1f} mm²); "
                    "duct is pinched/blocked"
                ),
            }]
    return []


def _check_connectivity(duct: Dict) -> List[Finding]:
    """connected must be true; false means a disconnected duct."""
    duct_id = duct.get("id", "<unknown>")
    if not duct.get("connected", True):
        return [{
            "code": "disconnected_flow",
            "detail": (
                f"duct '{duct_id}': duct is not connected — "
                "disconnected ducts are a hard reject"
            ),
        }]
    return []


def _check_gate_slot(duct: Dict) -> List[Finding]:
    """Printed gate slots are forbidden in dust collection paths."""
    duct_id = duct.get("id", "<unknown>")
    if duct.get("printed_gate_slot", False):
        return [{
            "code": "disconnected_flow",
            "detail": (
                f"duct '{duct_id}': printed gate slot detected — "
                "printed gate slots are prohibited in dust collection ducts"
            ),
        }]
    return []


def _check_pvc_as_duct(duct: Dict) -> List[Finding]:
    """PVC must not be used as the printed airflow duct."""
    duct_id = duct.get("id", "<unknown>")
    if duct.get("pvc_as_duct", False):
        return [{
            "code": "disconnected_flow",
            "detail": (
                f"duct '{duct_id}': PVC declared as the printed airflow duct — "
                "PVC must not be used as the printed duct; use a proper printed "
                "hose/duct/socket opening"
            ),
        }]
    return []


def _check_duct(duct: Dict) -> List[Finding]:
    """Run all checks for a single duct entry; return combined findings."""
    findings: List[Finding] = []
    findings.extend(_check_opening(duct))
    findings.extend(_check_cross_section(duct))
    findings.extend(_check_connectivity(duct))
    findings.extend(_check_gate_slot(duct))
    findings.extend(_check_pvc_as_duct(duct))
    return findings


# ---------------------------------------------------------------------------
# Stage entry point
# ---------------------------------------------------------------------------

def _load_descriptor(duct_path: Optional[str]) -> Tuple[Optional[Dict], Optional[Fragment]]:
    """
    Resolve the duct input. Returns (descriptor, early_fragment).

    When no duct descriptor is supplied (the engine sequences stages with only
    --in/--model/--out and this model has no dust-collection ducts), the stage
    DEGRADES to a non-blocking skip rather than hard-failing — a missing
    optional extra input is not a gate failure. On a load error the early
    fragment is a fail; otherwise early_fragment is None and descriptor is set.
    """
    if not duct_path:
        return None, {
            "stage": STAGE_NAME,
            "status": "skip",
            "detail": ("no duct descriptor supplied (--duct absent); "
                       "dust-airflow QA skipped"),
            "findings": [],
        }
    try:
        with open(duct_path) as fh:
            return json.load(fh), None
    except Exception as exc:
        return None, {
            "stage": STAGE_NAME,
            "status": "fail",
            "detail": f"failed to load duct JSON: {exc}",
            "findings": [{"code": "disconnected_flow", "detail": str(exc)}],
        }


def _assemble_fragment(findings: List[Finding], duct_count: int) -> Fragment:
    """Build the pass/fail fragment from accumulated findings."""
    if findings:
        detail = "; ".join(f["detail"] for f in findings)
        status = "fail"
    else:
        detail = (
            f"all {duct_count} duct(s) pass: full-size openings verified, "
            "cross-sections clear of pinches, all ducts connected, "
            "no printed gate slots, no PVC-as-duct"
        )
        status = "pass"
    return {
        "stage": STAGE_NAME,
        "status": status,
        "detail": detail,
        "findings": findings,
    }


def run(duct_path: Optional[str]) -> Fragment:
    """
    Execute all dust/airflow checks and return the stage fragment dict.

    This is the public API; main() wraps it for CLI use.
    """
    descriptor, early = _load_descriptor(duct_path)
    if early is not None:
        return early
    assert descriptor is not None

    ducts: List[Dict] = descriptor.get("ducts", [])

    all_findings: List[Finding] = []
    for duct in ducts:
        all_findings.extend(_check_duct(duct))

    return _assemble_fragment(all_findings, len(ducts))


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="autospec-fab dust leak/airflow QA stage",
    )
    parser.add_argument(
        "--in", dest="stl", required=True,
        help="path to STL file or directory (required for uniform stage CLI; "
             "dust verdict is driven by --duct descriptor, not raw geometry)",
    )
    parser.add_argument(
        "--duct", required=False, default=None,
        help="path to duct descriptor JSON (optional; absent → stage skips, "
             "for models with no dust-collection ducts)",
    )
    parser.add_argument(
        "--out", required=True,
        help="output path for stage fragment JSON",
    )
    parser.add_argument(
        "--model", required=False, default=None,
        help="per-model metadata sidecar (accepted for uniform stage CLI "
             "symmetry; dust verdict is driven by --duct, not metadata)",
    )
    args = parser.parse_args(argv)

    fragment = run(args.duct)

    try:
        with open(args.out, "w") as fh:
            json.dump(fragment, fh, indent=2)
    except Exception as exc:
        print(f"stage_dust: failed to write fragment: {exc}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())

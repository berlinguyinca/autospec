#!/usr/bin/env python3
"""
stage_cfd.py — CFD stage for autospec-fab (OpenFOAM).

Uniform stage CLI:
    stage_cfd.py --in <stl> --model <metadata.json> --out <fragment.json>
                 [--flow <flow.json>]

Behaviour:
  - flow_critical: false  → status skip, detail "not flow-critical".
  - solver absent from PATH → status skip, detail "solver not found …".
  - Geometry-hash cache   → hit returns cached fragment without re-running solver.
  - Cache miss            → build OpenFOAM case, run solver, parse result,
                            compare pressure-drop/velocity/stagnation to targets.
  - Target miss           → status fail + finding cfd_target_miss.
  - All targets met       → status pass.

Cache location:
  ${AUTOSPEC_FAB_CACHE_DIR}/<hash>.json
  (default: .autospec/fab/cfd-cache/ relative to cwd)

Exit codes:
  0 — harness success (verdict lives in fragment "status", not exit code).
  1 — harness/usage error (bad args, missing --out, etc.).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile

# Make sibling stage modules importable when run as a standalone script.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from openfoam_case import build_openfoam_case  # noqa: E402


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

_DEFAULT_MAX_PRESSURE_DROP_PA = 500.0
_DEFAULT_MIN_VELOCITY_M_S = 1.0
_DEFAULT_NO_STAGNATION = True
_SOLVER_NAME = "simpleFoam"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_finding(code: str, message: str) -> dict:
    return {"code": code, "message": message}


def _make_fragment(stage: str, status: str, detail: str, findings: list) -> dict:
    return {"stage": stage, "status": status, "detail": detail, "findings": findings}


def _load_json(path: str) -> dict:
    with open(path) as fh:
        return json.load(fh)


# ---------------------------------------------------------------------------
# Geometry hash
# ---------------------------------------------------------------------------

def _geometry_hash(stl_path: str, model: dict, flow: dict | None) -> str:
    """
    Hash the STL content plus print_orientation and flow spec.

    Any change to the geometry, orientation, or flow targets invalidates
    the cache entry.
    """
    h = hashlib.sha256()
    with open(stl_path, "rb") as fh:
        for chunk in iter(lambda: fh.read(65536), b""):
            h.update(chunk)
    key_fields = {
        "print_orientation": model.get("print_orientation", ""),
        "flow": flow or {},
    }
    h.update(json.dumps(key_fields, sort_keys=True).encode())
    return h.hexdigest()


# ---------------------------------------------------------------------------
# Cache helpers
# ---------------------------------------------------------------------------

def _cache_dir() -> str:
    override = os.environ.get("AUTOSPEC_FAB_CACHE_DIR")
    if override:
        return override
    return os.path.join(".autospec", "fab", "cfd-cache")


def _cache_read(cache_root: str, digest: str) -> dict | None:
    path = os.path.join(cache_root, f"{digest}.json")
    if not os.path.exists(path):
        return None
    try:
        with open(path) as fh:
            return json.load(fh)
    except (OSError, json.JSONDecodeError):
        return None


def _cache_write(cache_root: str, digest: str, fragment: dict) -> None:
    os.makedirs(cache_root, exist_ok=True)
    path = os.path.join(cache_root, f"{digest}.json")
    with open(path, "w") as fh:
        json.dump(fragment, fh, indent=2)
        fh.write("\n")


# ---------------------------------------------------------------------------
# Solver runner
# ---------------------------------------------------------------------------

def _run_solver(case_dir: str) -> tuple[bool, str]:
    """
    Run the OpenFOAM solver (simpleFoam or shim) on case_dir.
    Returns (success, stderr_text).
    """
    solver_bin = shutil.which(_SOLVER_NAME)
    if solver_bin is None:
        return False, f"{_SOLVER_NAME} not found on PATH"
    try:
        result = subprocess.run(
            [solver_bin],
            capture_output=True,
            text=True,
            cwd=case_dir,
        )
    except OSError as exc:
        return False, str(exc)
    if result.returncode != 0:
        return False, (result.stdout + result.stderr).strip()
    return True, ""


# ---------------------------------------------------------------------------
# Result parser
# ---------------------------------------------------------------------------

def _parse_cfd_results(case_dir: str) -> dict | None:
    """
    Parse CFD metrics from the solver output file in case_dir.

    The shim (and a real solver wrapper) writes a file named ``cfd_results``
    in the case directory containing lines of the form:
        PRESSURE_DROP <value_pa>
        MIN_VELOCITY  <value_m_s>
        STAGNATION    <0|1>

    Returns a dict with keys pressure_drop_pa, min_velocity_m_s, stagnation.
    Returns None if the file is absent or malformed.
    """
    result_path = os.path.join(case_dir, "cfd_results")
    if not os.path.exists(result_path):
        return None
    results: dict = {}
    with open(result_path) as fh:
        for line in fh:
            parts = line.strip().split()
            if len(parts) < 2:
                continue
            key = parts[0].upper()
            try:
                val = float(parts[1])
            except ValueError:
                continue
            if key == "PRESSURE_DROP":
                results["pressure_drop_pa"] = val
            elif key == "MIN_VELOCITY":
                results["min_velocity_m_s"] = val
            elif key == "STAGNATION":
                results["stagnation"] = bool(int(val))
    if not results:
        return None
    return results


# ---------------------------------------------------------------------------
# Target evaluation
# ---------------------------------------------------------------------------

def _evaluate_targets(results: dict, flow: dict) -> list[str]:
    """
    Compare parsed CFD metrics to flow targets.
    Returns a list of human-readable miss descriptions (empty = all pass).
    """
    misses: list[str] = []

    max_dp = float(flow.get("max_pressure_drop_pa", _DEFAULT_MAX_PRESSURE_DROP_PA))
    min_vel = float(flow.get("min_velocity_m_s", _DEFAULT_MIN_VELOCITY_M_S))
    no_stagnation = bool(flow.get("no_stagnation", _DEFAULT_NO_STAGNATION))

    dp = results.get("pressure_drop_pa")
    if dp is not None and dp > max_dp:
        misses.append(
            f"pressure_drop {dp:.1f} Pa > max {max_dp:.1f} Pa"
        )

    vel = results.get("min_velocity_m_s")
    if vel is not None and vel < min_vel:
        misses.append(
            f"min_velocity {vel:.2f} m/s < required {min_vel:.2f} m/s"
        )

    if no_stagnation and results.get("stagnation", False):
        misses.append("stagnation zone detected")

    return misses


# ---------------------------------------------------------------------------
# Stage runner
# ---------------------------------------------------------------------------

def _load_flow_spec(flow_path: str | None) -> dict:
    """Load the flow spec JSON if present, else return an empty dict."""
    if flow_path and os.path.exists(flow_path):
        return _load_json(flow_path)
    return {}


def _solve_case(stl_path: str, model: dict, flow: dict) -> tuple[bool, str, dict | None]:
    """
    Build the OpenFOAM case in a temp dir, run the solver, and parse results.
    Returns (success, err_msg, results). results is None on parse failure.
    """
    with tempfile.TemporaryDirectory(prefix="cfd_run_") as work_dir:
        case_dir = build_openfoam_case(stl_path, model, flow, work_dir)
        success, err_msg = _run_solver(case_dir)
        if not success:
            return False, err_msg, None
        return True, "", _parse_cfd_results(case_dir)


def _result_fragment(results: dict | None, flow: dict) -> dict:
    """Map parsed CFD results + flow targets to a stage fragment."""
    if results is None:
        return _make_fragment(
            "cfd", "fail",
            "solver ran but CFD results could not be parsed from output",
            [_make_finding("cfd_target_miss", "CFD result parse failure")],
        )

    misses = _evaluate_targets(results, flow)
    if misses:
        detail = "CFD target miss: " + "; ".join(misses)
        return _make_fragment(
            "cfd", "fail", detail,
            [_make_finding("cfd_target_miss", detail)],
        )

    dp = results.get("pressure_drop_pa", 0.0)
    vel = results.get("min_velocity_m_s", 0.0)
    detail = (
        f"CFD targets met: pressure_drop={dp:.1f} Pa, "
        f"min_velocity={vel:.2f} m/s"
    )
    return _make_fragment("cfd", "pass", detail, [])


def run_cfd_stage(
    stl_path: str,
    model_path: str,
    flow_path: str | None,
    out_path: str,
) -> dict:
    """
    Run the CFD stage. Writes the fragment to out_path. Returns the fragment.
    """
    model = _load_json(model_path)

    if not model.get("flow_critical", False):
        return _make_fragment("cfd", "skip", "not flow-critical", [])

    flow = _load_flow_spec(flow_path)

    # --- Check solver availability BEFORE cache lookup ---
    if shutil.which(_SOLVER_NAME) is None:
        return _make_fragment(
            "cfd", "skip",
            f"{_SOLVER_NAME} not found on PATH; real solver deferred to container",
            [],
        )

    # --- Geometry-hash cache lookup ---
    digest = _geometry_hash(stl_path, model, flow)
    cache_root = _cache_dir()
    cached = _cache_read(cache_root, digest)
    if cached is not None:
        return cached

    # --- Cache miss: build case, run solver, evaluate targets ---
    success, err_msg, results = _solve_case(stl_path, model, flow)
    if not success:
        fragment = _make_fragment(
            "cfd", "fail",
            f"solver execution failed: {err_msg[:200]}",
            [_make_finding("cfd_target_miss", f"solver failed: {err_msg[:200]}")],
        )
    else:
        fragment = _result_fragment(results, flow)

    _cache_write(cache_root, digest, fragment)
    return fragment


# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------

def main(argv=None):
    parser = argparse.ArgumentParser(
        description="autospec-fab CFD stage: OpenFOAM flow analysis"
    )
    parser.add_argument(
        "--in", dest="in_path", required=True,
        help="Input STL file",
    )
    parser.add_argument(
        "--model", dest="model_path", required=True,
        help="Per-model metadata JSON sidecar",
    )
    parser.add_argument(
        "--out", required=True,
        help="Output fragment JSON path",
    )
    parser.add_argument(
        "--flow", dest="flow_path", default=None,
        help="Flow specification JSON (targets: max_pressure_drop_pa, min_velocity_m_s, no_stagnation)",
    )

    args = parser.parse_args(argv)

    fragment = run_cfd_stage(
        stl_path=args.in_path,
        model_path=args.model_path,
        flow_path=args.flow_path,
        out_path=args.out,
    )

    out_dir = os.path.dirname(os.path.abspath(args.out))
    os.makedirs(out_dir, exist_ok=True)
    with open(args.out, "w") as fh:
        json.dump(fragment, fh, indent=2)
        fh.write("\n")

    sys.exit(0)


if __name__ == "__main__":
    main()

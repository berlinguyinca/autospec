#!/usr/bin/env python3
"""
stage_fea.py — structural FEA stage for autospec-fab (CalculiX).

Uniform stage CLI:
    stage_fea.py --in <stl> --model <metadata.json> --out <fragment.json>
                 [--load <load.json>]

Behaviour:
  - load_critical: false / ccx absent → status skip (no fea_results).
  - Geometry-hash cache hit → cached fragment (no re-run).
  - Cache miss → build anisotropic ccx deck, run ccx, parse safety factor,
    compare to minimum, write cache entry. Pass/fail fragments carry a
    structured fea_results {safety_factor, required_min, status}.

Cache: ${AUTOSPEC_FAB_CACHE_DIR}/<hash>.json (default .autospec/fab/fea-cache/).
Exit 0 on harness success (verdict lives in fragment "status"); 1 on usage error.
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
from typing import Any


# ---------------------------------------------------------------------------
# Material anisotropy tables
# ---------------------------------------------------------------------------

# Per-axis Young's modulus scale factors vs the isotropic base value. FDM is
# strongest in the X/Y print plane and weakest in Z (between-layer bonding);
# the ratios encode that orientation dependency for typical FDM PETG/PLA/ABS.
_ORIENTATION_SCALE: dict[str, dict[str, float]] = {
    # orientation → {E_x, E_y, E_z} scale factors
    "flat":    {"E_x": 1.00, "E_y": 1.00, "E_z": 0.55},
    "upright": {"E_x": 0.55, "E_y": 1.00, "E_z": 1.00},
    "side":    {"E_x": 1.00, "E_y": 0.55, "E_z": 1.00},
}
_DEFAULT_ORIENTATION_SCALE = {"E_x": 0.80, "E_y": 0.80, "E_z": 0.80}

# Base isotropic Young's modulus (MPa) and Poisson's ratio per material.
_MATERIAL_BASE: dict[str, dict[str, float]] = {
    "PETG": {"E_mpa": 2100.0, "nu": 0.38},
    "PLA":  {"E_mpa": 3500.0, "nu": 0.36},
    "ABS":  {"E_mpa": 2300.0, "nu": 0.35},
    "ASA":  {"E_mpa": 2200.0, "nu": 0.35},
    "PCTG": {"E_mpa": 2000.0, "nu": 0.38},
}
_DEFAULT_MATERIAL_BASE = {"E_mpa": 2000.0, "nu": 0.38}

_DEFAULT_SAFETY_MIN = 2.0


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

def _geometry_hash(stl_path: str, model: dict, load: dict | None) -> str:
    """Hash STL content + material + print_orientation + load spec (cache key)."""
    h = hashlib.sha256()
    with open(stl_path, "rb") as fh:
        for chunk in iter(lambda: fh.read(65536), b""):
            h.update(chunk)
    # Include fields that affect the analysis result
    key_fields = {
        "material": model.get("material", ""),
        "print_orientation": model.get("print_orientation", ""),
        "load": load or {},
    }
    h.update(json.dumps(key_fields, sort_keys=True).encode())
    return h.hexdigest()


# ---------------------------------------------------------------------------
# Cache helpers
# ---------------------------------------------------------------------------

def _cache_dir(env_override: str | None = None) -> str:
    if env_override:
        return env_override
    override = os.environ.get("AUTOSPEC_FAB_CACHE_DIR")
    if override:
        return override
    return os.path.join(".autospec", "fab", "fea-cache")


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
# Anisotropic CalculiX deck builder
# ---------------------------------------------------------------------------

def _build_ccx_deck(
    stl_path: str,
    model: dict,
    load: dict,
    work_dir: str,
    job_name: str = "fea_job",
) -> str:
    """Write a minimal CalculiX input deck (.inp); return the job base path.

    The deck encodes per-axis anisotropic elastic constants derived from the
    material + print_orientation on a single 8-node brick element (the goal is
    to exercise solver integration + orientation encoding, not production FEA).
    """
    mat_name = model.get("material", "GENERIC")
    orientation = model.get("print_orientation", "flat")
    base = _MATERIAL_BASE.get(mat_name, _DEFAULT_MATERIAL_BASE)
    scale = _ORIENTATION_SCALE.get(orientation, _DEFAULT_ORIENTATION_SCALE)

    E = (base["E_mpa"] * scale["E_x"],
         base["E_mpa"] * scale["E_y"],
         base["E_mpa"] * scale["E_z"])
    nu = base["nu"]
    G = base["E_mpa"] / (2.0 * (1.0 + nu))  # shear modulus (isotropic approx)

    force_n = load.get("force_n", 10.0)
    direction = load.get("direction", [0, 0, -1])

    inp_path = os.path.join(work_dir, f"{job_name}.inp")
    with open(inp_path, "w") as fh:
        _write_ccx_inp(fh, mat_name, orientation, E, nu, G, force_n, direction)
    return os.path.join(work_dir, job_name)


# Unit-cube C3D8 nodes: (id, x, y, z). Bottom face 1-4 fixed; load on top 5-8.
_CUBE_NODES = (
    (1, 0.0, 0.0, 0.0), (2, 1.0, 0.0, 0.0), (3, 1.0, 1.0, 0.0),
    (4, 0.0, 1.0, 0.0), (5, 0.0, 0.0, 1.0), (6, 1.0, 0.0, 1.0),
    (7, 1.0, 1.0, 1.0), (8, 0.0, 1.0, 1.0),
)


def _write_ccx_inp(fh, mat_name, orientation, E, nu, G, force_n, direction):
    """Write the .inp deck body (one 8-node brick, orthotropic material)."""
    E_x, E_y, E_z = E
    fh.write("** autospec-fab FEA deck\n")
    fh.write(f"** Material: {mat_name}  Orientation: {orientation}\n")
    fh.write(f"** E_x={E_x:.1f} E_y={E_y:.1f} E_z={E_z:.1f} MPa\n**\n")
    fh.write("*NODE\n")
    for n, x, y, z in _CUBE_NODES:
        fh.write(f"{n}, {x}, {y}, {z}\n")
    fh.write("*ELEMENT, TYPE=C3D8, ELSET=EALL\n1, 1, 2, 3, 4, 5, 6, 7, 8\n")
    # Orthotropic: E1,E2,E3, nu12,nu13,nu23, G12,G13 then G23 on next line.
    fh.write(f"*MATERIAL, NAME={mat_name}\n")
    fh.write("*ELASTIC, TYPE=ENGINEERING CONSTANTS\n")
    fh.write(f"{E_x:.2f}, {E_y:.2f}, {E_z:.2f}, "
             f"{nu:.4f}, {nu:.4f}, {nu:.4f}, {G:.2f}, {G:.2f}\n")
    fh.write(f"{G:.2f}\n")
    fh.write(f"*SOLID SECTION, ELSET=EALL, MATERIAL={mat_name}\n")
    fh.write("*BOUNDARY\n")
    for nid in range(1, 5):
        fh.write(f"{nid}, 1, 3, 0.0\n")
    comps = (force_n * direction[0] / 4.0, force_n * direction[1] / 4.0,
             force_n * direction[2] / 4.0)
    fh.write("*CLOAD\n")
    for nid in range(5, 9):
        for axis, val in enumerate(comps, start=1):
            if val != 0.0:
                fh.write(f"{nid}, {axis}, {val:.4f}\n")
    fh.write("*STEP\n*STATIC\n*EL PRINT, ELSET=EALL\nS\n")
    fh.write("*NODE PRINT, NSET=NALL\nU\n*END STEP\n")


# ---------------------------------------------------------------------------
# ccx runner + result parser
# ---------------------------------------------------------------------------

def _run_ccx(job_base: str) -> tuple[bool, str]:
    """Run ccx on job_base.inp; return (success, stderr_text).

    Invoked as: ccx <job_base> (no extension — ccx appends .inp/.dat).
    """
    ccx_bin = shutil.which("ccx")
    if ccx_bin is None:
        return False, "ccx not found on PATH"
    work_dir = os.path.dirname(job_base)
    job_name = os.path.basename(job_base)
    try:
        result = subprocess.run(
            [ccx_bin, job_name],
            capture_output=True,
            text=True,
            cwd=work_dir,
        )
    except OSError as exc:
        return False, str(exc)
    if result.returncode != 0:
        return False, (result.stdout + result.stderr).strip()
    return True, ""


def _parse_safety_factor(job_base: str) -> float | None:
    """Parse the safety factor from the ccx .dat output file.

    The shim/real wrapper writes `SAFETY_FACTOR <value>`; absent → None.
    """
    dat_path = job_base + ".dat"
    if not os.path.exists(dat_path):
        return None
    with open(dat_path) as fh:
        for line in fh:
            stripped = line.strip()
            if stripped.upper().startswith("SAFETY_FACTOR"):
                parts = stripped.split()
                if len(parts) >= 2:
                    try:
                        return float(parts[1])
                    except ValueError:
                        pass
    return None


# ---------------------------------------------------------------------------
# Stage runner
# ---------------------------------------------------------------------------

def run_fea_stage(
    stl_path: str,
    model_path: str,
    load_path: str | None,
    out_path: str,
) -> dict:
    """
    Run the FEA stage.  Writes the fragment to out_path.  Returns the fragment.
    """
    model = _load_json(model_path)

    # --- Non-load-critical: skip immediately ---
    if not model.get("load_critical", False):
        return _make_fragment("fea", "skip", "not load-critical", [])

    # --- Load spec ---
    load: dict[str, Any] = {}
    if load_path and os.path.exists(load_path):
        load = _load_json(load_path)
    safety_min = float(load.get("safety_factor_min", _DEFAULT_SAFETY_MIN))

    # ccx absent -> skip (real solver deferred); cache is irrelevant.
    ccx_bin = shutil.which("ccx")
    if ccx_bin is None:
        return _make_fragment("fea", "skip", "ccx not found on PATH; real solver deferred to container", [])

    # --- Geometry-hash cache lookup ---
    digest = _geometry_hash(stl_path, model, load)
    cache_root = _cache_dir()
    cached = _cache_read(cache_root, digest)
    if cached is not None:
        return cached

    # --- Cache miss: build deck, run ccx, parse result ---
    success, err_msg, safety_factor = _run_and_parse(stl_path, model, load)
    if not success:
        fragment = _make_fragment(
            "fea", "fail",
            f"ccx execution failed: {err_msg[:200]}",
            [_make_finding("fea_below_safety", f"ccx failed: {err_msg[:200]}")],
        )
    else:
        fragment = _safety_fragment(safety_factor, safety_min, model)
    _cache_write(cache_root, digest, fragment)
    return fragment


def _run_and_parse(stl_path, model, load):
    """Build the ccx deck, run ccx, parse the safety factor.

    Returns (success, err_msg, safety_factor). safety_factor is None on a parse
    failure (success stays True so the caller emits the parse-failure verdict).
    """
    with tempfile.TemporaryDirectory(prefix="fea_run_") as work_dir:
        job_base = _build_ccx_deck(stl_path, model, load, work_dir)
        success, err_msg = _run_ccx(job_base)
        if not success:
            return False, err_msg, None
        return True, "", _parse_safety_factor(job_base)


def _safety_fragment(safety_factor, safety_min, model):
    """Map a parsed safety factor to a pass/fail fragment + fea_results.

    safety_factor is None -> parse-failure fail (no fea_results numbers).
    """
    if safety_factor is None:
        return _make_fragment(
            "fea", "fail",
            "ccx ran but safety factor could not be parsed from .dat output",
            [_make_finding("fea_below_safety", "Safety factor parse failure")],
        )
    cmp = "<" if safety_factor < safety_min else "≥"
    detail = (
        f"Safety factor {safety_factor:.3f} {cmp} required {safety_min:.1f} "
        f"(material={model.get('material')}, "
        f"orientation={model.get('print_orientation')})"
    )
    if safety_factor < safety_min:
        fragment = _make_fragment(
            "fea", "fail", detail,
            [_make_finding("fea_below_safety", detail)])
    else:
        fragment = _make_fragment("fea", "pass", detail, [])
    fragment["fea_results"] = {
        "safety_factor": safety_factor,
        "required_min": safety_min,
        "status": fragment["status"],
    }
    return fragment


# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------

def main(argv=None):
    parser = argparse.ArgumentParser(
        description="autospec-fab FEA stage: CalculiX structural analysis"
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
        "--load", dest="load_path", default=None,
        help="Load specification JSON (force, direction, safety_factor_min)",
    )

    args = parser.parse_args(argv)

    fragment = run_fea_stage(
        stl_path=args.in_path,
        model_path=args.model_path,
        load_path=args.load_path,
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

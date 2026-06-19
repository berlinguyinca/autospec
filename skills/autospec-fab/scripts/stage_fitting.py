#!/usr/bin/env python3
"""
stage_fitting.py — vacuum-fitting QA stage for autospec-fab.

Uniform stage CLI:
    stage_fitting.py --in <stl|dir> --model <metadata.json> --out <fragment.json>

Checks performed (per NPT port in model metadata):
  1. RADIAL CLEARANCE   — port.clearance_mm must be ≥ 5 mm (or ≥ a larger
                          documented envelope when clearance_mm > 5).
                          Fails with code_health blocked_npt when < 5.
  2. TAP ACCESS         — port must declare tap_access: true.
                          Fails with blocked_npt when false or absent.
  3. THREAD DEPTH       — NPT port must declare thread_depth_mm ≥ MIN_THREAD_MM.
                          Fails with blocked_npt when missing or too shallow.
  4. NARROW TOWER       — port must NOT sit on a narrow standalone tower/boss.
                          Detection is two-layer:
                          a) metadata flag: port.standalone_tower == true → reject.
                          b) geometry heuristic (stdlib STL probe): bounding-box
                             aspect ratio height/min(width,depth) > TOWER_RATIO.
                             A compact integrated body stays well below the
                             threshold; a thin ear/boss exceeds it.
                          Fails with blocked_npt when either layer triggers.

Non-NPT ports (hose, socket) are only checked for radial clearance (rule 1).

Exit codes:
  0  — harness success (gate verdict is in fragment "status", not exit code).
  1  — harness/usage error (bad args, unreadable files, etc.).

Trimesh is optional: lazy-imported for robustness; the stdlib ASCII parser is
the default path exercised by tests and used on bare python3.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import sys
from typing import Dict, List, Optional, Tuple

# ---------------------------------------------------------------------------
# Optional trimesh
# ---------------------------------------------------------------------------
try:
    import trimesh as _trimesh  # type: ignore
except ImportError:
    _trimesh = None  # type: ignore

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

MIN_CLEARANCE_MM: float = 5.0          # radial free clearance floor (mm)
MIN_THREAD_DEPTH_MM: float = 5.0       # usable NPT thread depth floor (mm)
# Height-to-min-footprint ratio above which a body is classified as a
# narrow tower/boss.  A 4×4×40 mm pin → ratio 10; a 20×20×20 mm cube → 1.
# Threshold set conservatively: integrated port bosses on real models rarely
# exceed 3; narrow ear/towers typically exceed 5.
TOWER_RATIO: float = 4.0

# Stage name emitted in every fragment
STAGE_NAME = "vacuum-fitting"

# ---------------------------------------------------------------------------
# Types
# ---------------------------------------------------------------------------
Triangle = Tuple[
    Tuple[float, float, float],
    Tuple[float, float, float],
    Tuple[float, float, float],
]
Finding = Dict[str, str]
Fragment = Dict

# ---------------------------------------------------------------------------
# ASCII STL parser (stdlib — same pattern as stage-geometry.py)
# ---------------------------------------------------------------------------

_FACET_RE = re.compile(r"facet\s+normal", re.IGNORECASE)
_VERTEX_RE = re.compile(
    r"vertex\s+([\-+]?[0-9]*\.?[0-9]+(?:[eE][\-+]?[0-9]+)?)"
    r"\s+([\-+]?[0-9]*\.?[0-9]+(?:[eE][\-+]?[0-9]+)?)"
    r"\s+([\-+]?[0-9]*\.?[0-9]+(?:[eE][\-+]?[0-9]+)?)",
    re.IGNORECASE,
)


def _parse_stl_ascii(path: str) -> List[Triangle]:
    """Parse ASCII STL → list of triangles (each: 3 (x,y,z) tuples)."""
    triangles: List[Triangle] = []
    with open(path, "r", errors="replace") as fh:
        content = fh.read()
    for block in _FACET_RE.split(content)[1:]:
        verts = _VERTEX_RE.findall(block)
        if len(verts) >= 3:
            tri = tuple(
                (float(x), float(y), float(z)) for x, y, z in verts[:3]
            )
            triangles.append(tri)  # type: ignore[arg-type]
    return triangles


def _parse_stl(path: str) -> Optional[List[Triangle]]:
    """
    Parse STL at *path*.  Tries trimesh first (binary STL support) when
    available; falls back to stdlib ASCII parser.  Returns None on failure.
    """
    if _trimesh is not None:
        try:
            mesh = _trimesh.load_mesh(path)
            tris: List[Triangle] = []
            for face in mesh.faces:
                v0 = tuple(float(c) for c in mesh.vertices[face[0]])
                v1 = tuple(float(c) for c in mesh.vertices[face[1]])
                v2 = tuple(float(c) for c in mesh.vertices[face[2]])
                tris.append((v0, v1, v2))  # type: ignore[arg-type]
            return tris or None
        except Exception:
            pass
    try:
        tris = _parse_stl_ascii(path)
        return tris or None
    except Exception:
        return None


# ---------------------------------------------------------------------------
# Geometry probe — bounding-box aspect ratio
# ---------------------------------------------------------------------------

def _bounding_box(triangles: List[Triangle]) -> Tuple[float, float, float]:
    """Return (x_range, y_range, z_range) bounding-box extents in mm."""
    xs = [v[0] for tri in triangles for v in tri]
    ys = [v[1] for tri in triangles for v in tri]
    zs = [v[2] for tri in triangles for v in tri]
    return (
        max(xs) - min(xs),
        max(ys) - min(ys),
        max(zs) - min(zs),
    )


def _is_narrow_tower(triangles: List[Triangle]) -> bool:
    """
    Heuristic narrow-tower detector (stdlib geometry probe).

    Computes the bounding-box and tests whether the tallest dimension
    divided by the smaller of the two footprint dimensions exceeds
    TOWER_RATIO.  This flags a 4×4×40 mm ear (ratio=10) while a
    20×20×20 mm integrated boss (ratio=1) stays below the threshold.

    The heuristic is intentionally simple and documented: it cannot
    replace a full sectional analysis (no trimesh required), but it
    correctly gates the canonical narrow-tower failure mode per the
    STL Modeling Rules.
    """
    if not triangles:
        return False
    dx, dy, dz = _bounding_box(triangles)
    extents = sorted([dx, dy, dz])  # ascending: smallest, mid, largest
    height = extents[2]
    footprint_min = extents[0]  # smallest lateral dimension
    if footprint_min < 1e-6:
        return True  # degenerate — treat as tower
    return (height / footprint_min) > TOWER_RATIO


# ---------------------------------------------------------------------------
# Per-port checks
# ---------------------------------------------------------------------------

def _check_port(
    port: dict,
    triangles: Optional[List[Triangle]],
) -> List[Finding]:
    """
    Run all applicable checks for a single port dict.

    Returns a (possibly empty) list of Finding dicts; an empty list means
    the port passed all checks.
    """
    findings: List[Finding] = []
    port_id = port.get("id", "<unknown>")
    port_type = port.get("type", "")

    # --- Rule 1: radial clearance (all port types) -------------------------
    clearance = port.get("clearance_mm")
    if clearance is None or clearance < MIN_CLEARANCE_MM:
        actual = clearance if clearance is not None else "not declared"
        findings.append({
            "code": "blocked_npt",
            "port": port_id,
            "detail": (
                f"port '{port_id}' radial clearance {actual} mm "
                f"< required {MIN_CLEARANCE_MM} mm"
            ),
        })

    # Rules 2-4 apply to NPT ports only
    if port_type != "npt":
        return findings

    # --- Rule 2: tap access ------------------------------------------------
    if not port.get("tap_access", False):
        findings.append({
            "code": "blocked_npt",
            "port": port_id,
            "detail": (
                f"NPT port '{port_id}' does not declare tap_access: true"
            ),
        })

    # --- Rule 3: thread depth ----------------------------------------------
    thread_depth = port.get("thread_depth_mm")
    if thread_depth is None or thread_depth < MIN_THREAD_DEPTH_MM:
        actual = thread_depth if thread_depth is not None else "not declared"
        findings.append({
            "code": "blocked_npt",
            "port": port_id,
            "detail": (
                f"NPT port '{port_id}' thread_depth_mm {actual} "
                f"< required {MIN_THREAD_DEPTH_MM} mm"
            ),
        })

    # --- Rule 4: narrow tower ----------------------------------------------
    # Layer a: metadata flag
    if port.get("standalone_tower", False):
        findings.append({
            "code": "blocked_npt",
            "port": port_id,
            "detail": (
                f"NPT port '{port_id}' is on a standalone tower/boss "
                "(standalone_tower declared in metadata)"
            ),
        })
    # Layer b: geometry heuristic (only when metadata flag is absent and
    # triangles are available — avoids double-counting)
    elif triangles is not None and _is_narrow_tower(triangles):
        dx, dy, dz = _bounding_box(triangles)
        extents = sorted([dx, dy, dz])
        ratio = extents[2] / max(extents[0], 1e-6)
        findings.append({
            "code": "blocked_npt",
            "port": port_id,
            "detail": (
                f"NPT port '{port_id}' geometry suggests a narrow tower/boss "
                f"(height/min-footprint ratio {ratio:.1f} > threshold {TOWER_RATIO})"
            ),
        })

    return findings


# ---------------------------------------------------------------------------
# Stage entry point
# ---------------------------------------------------------------------------

def _resolve_stl(path: str) -> Optional[str]:
    """
    Resolve *path* to a single STL file path.
    If *path* is a directory, return the first *.stl found (case-insensitive).
    """
    if os.path.isfile(path):
        return path
    if os.path.isdir(path):
        for name in sorted(os.listdir(path)):
            if name.lower().endswith(".stl"):
                return os.path.join(path, name)
    return None


def run(stl_path: str, model_path: str) -> Fragment:
    """
    Execute all fitting checks and return the stage fragment dict.

    This is the public API; main() wraps it for CLI use.
    """
    all_findings: List[Finding] = []

    # Load model metadata
    try:
        with open(model_path) as fh:
            model = json.load(fh)
    except Exception as exc:
        return {
            "stage": STAGE_NAME,
            "status": "fail",
            "detail": f"failed to load model metadata: {exc}",
            "findings": [{"code": "blocked_npt", "detail": str(exc)}],
        }

    ports: List[dict] = model.get("ports", [])

    # Load STL (best-effort; None if unavailable)
    resolved = _resolve_stl(stl_path)
    triangles: Optional[List[Triangle]] = None
    if resolved:
        triangles = _parse_stl(resolved)

    for port in ports:
        port_findings = _check_port(port, triangles)
        all_findings.extend(port_findings)

    if all_findings:
        codes = sorted({f["code"] for f in all_findings})
        port_ids = sorted({f.get("port", "") for f in all_findings if f.get("port")})
        detail = (
            f"fitting QA failed on port(s) {port_ids}: "
            + "; ".join(f["detail"] for f in all_findings)
        )
        status = "fail"
    else:
        port_count = len(ports)
        detail = f"all {port_count} port(s) passed fitting QA"
        status = "pass"

    return {
        "stage": STAGE_NAME,
        "status": status,
        "detail": detail,
        "findings": all_findings,
    }


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="autospec-fab vacuum-fitting QA stage",
    )
    parser.add_argument("--in", dest="stl", required=True,
                        help="path to STL file or directory containing STL")
    parser.add_argument("--model", required=True,
                        help="path to model metadata JSON sidecar")
    parser.add_argument("--out", required=True,
                        help="output path for stage fragment JSON")
    args = parser.parse_args(argv)

    fragment = run(args.stl, args.model)

    try:
        with open(args.out, "w") as fh:
            json.dump(fragment, fh, indent=2)
    except Exception as exc:
        print(f"stage_fitting: failed to write fragment: {exc}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())

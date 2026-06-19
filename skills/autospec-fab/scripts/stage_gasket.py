#!/usr/bin/env python3
"""
stage_gasket.py — gasket leak-sim QA stage for autospec-fab.

Uniform stage CLI:
    stage_gasket.py --in <stl|dir> --model <metadata.json> --out <fragment.json>

Checks performed (per gasket in model metadata):
  1. SURROUNDING WALL      — gasket.surrounding_wall_mm must be ≥ 5 mm.
                             Fails with code_health exposed_gasket (thin wall)
                             when < 5 or not declared.
  2. SEAL-FACE CONTINUITY  — gasket.seal_face_continuous must be true.
                             Fails with exposed_gasket (broken seal face) when
                             false or absent.
  3. EXPOSED SIDE          — gasket.exposed_side must be false (or absent).
                             Fails with exposed_gasket (exposed side) when true.

All three checks use declared metadata fields as the primary signal.
The stdlib STL geometry probe is loaded opportunistically for cross-checking
but never required; trimesh is optional (lazy try/except).

Exit codes:
  0  — harness success (gate verdict is in fragment "status", not exit code).
  1  — harness/usage error (bad args, unreadable files, etc.).
"""

from __future__ import annotations

import argparse
import json
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

MIN_SURROUNDING_WALL_MM: float = 5.0  # minimum plastic outside groove (mm)
STAGE_NAME = "gasket"

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
# ASCII STL parser (stdlib — same pattern as stage_fitting.py)
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
# Geometry probe — bounding-box extents
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


# ---------------------------------------------------------------------------
# Per-gasket checks
# ---------------------------------------------------------------------------

def _check_gasket(
    gasket: dict,
    _triangles: Optional[List[Triangle]],
) -> List[Finding]:
    """
    Run all checks for a single gasket dict.

    Returns a (possibly empty) list of Finding dicts; empty means all pass.
    The triangles argument is available for optional geometry cross-checks
    but none are required — metadata fields are the authoritative signal.
    """
    findings: List[Finding] = []
    gid = gasket.get("id", "<unknown>")

    # --- Rule 1: surrounding wall ≥ 5 mm -----------------------------------
    wall = gasket.get("surrounding_wall_mm")
    if wall is None or wall < MIN_SURROUNDING_WALL_MM:
        actual = wall if wall is not None else "not declared"
        findings.append({
            "code": "exposed_gasket",
            "gasket": gid,
            "detail": (
                f"gasket '{gid}' surrounding_wall_mm {actual} mm "
                f"< required {MIN_SURROUNDING_WALL_MM} mm (thin wall)"
            ),
        })

    # --- Rule 2: seal-face continuity --------------------------------------
    if not gasket.get("seal_face_continuous", False):
        findings.append({
            "code": "exposed_gasket",
            "gasket": gid,
            "detail": (
                f"gasket '{gid}' seal_face_continuous is false or absent "
                "(broken seal face — pressure boundary not closed)"
            ),
        })

    # --- Rule 3: exposed side ----------------------------------------------
    if gasket.get("exposed_side", False):
        findings.append({
            "code": "exposed_gasket",
            "gasket": gid,
            "detail": (
                f"gasket '{gid}' exposed_side is true "
                "(groove side open to exterior — rejected)"
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
    Execute all gasket checks and return the stage fragment dict.

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
            "findings": [{"code": "exposed_gasket", "detail": str(exc)}],
        }

    gaskets: List[dict] = model.get("gaskets", [])

    # Load STL best-effort (None if unavailable) for optional cross-checks
    resolved = _resolve_stl(stl_path)
    triangles: Optional[List[Triangle]] = None
    if resolved:
        triangles = _parse_stl(resolved)

    for gasket in gaskets:
        gasket_findings = _check_gasket(gasket, triangles)
        all_findings.extend(gasket_findings)

    if all_findings:
        gasket_ids = sorted({f.get("gasket", "") for f in all_findings if f.get("gasket")})
        detail = (
            f"gasket QA failed on gasket(s) {gasket_ids}: "
            + "; ".join(f["detail"] for f in all_findings)
        )
        status = "fail"
    else:
        gasket_count = len(gaskets)
        detail = f"all {gasket_count} gasket(s) passed leak-sim QA"
        status = "pass"

    return {
        "stage": STAGE_NAME,
        "status": status,
        "detail": detail,
        "findings": all_findings,
    }


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="autospec-fab gasket leak-sim QA stage",
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
        print(f"stage_gasket: failed to write fragment: {exc}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())

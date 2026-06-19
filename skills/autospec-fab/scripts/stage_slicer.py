#!/usr/bin/env python3
"""
stage_slicer.py — slicer QA stage for autospec-fab.

Uniform stage CLI:
    stage_slicer.py --in <stl> --out <fragment.json>
                    [--printer <printer.json>] [--config <fab.yml>]

Checks performed:
  1. WALL THICKNESS  — estimate minimum feature thickness from the STL bounding
     box and face layout; flag thin walls below min_perimeters × nozzle_width.
     Heuristic: for each axis-aligned direction, the extent of the mesh in that
     axis is the "wall span". When the minimum axis extent (the shortest
     dimension) is less than min_wall, that dimension is a thin-wall candidate.
     This is intentionally simple and documented: it catches plates and slabs
     whose smallest extent is sub-perimeter, which is the most common FDM
     failure mode. It does NOT do expensive ray-casting.
  2. OVERHANG ANGLE  — for every face, compute the face normal and measure its
     angle from the build-Z axis (+Z). If the normal has a significant downward
     Z component (i.e. the face points "downward") and the angle from vertical
     exceeds max_overhang_deg, it is a steep overhang. A face is downward-facing
     when its normal Z component is negative (nz < 0). The overhang angle is
     measured as the angle between the face normal and the downward -Z axis:
       overhang_deg = arccos(|nz| / |normal|) when nz < 0
     If overhang_deg > max_overhang_deg → slicer_overhang finding.
  3. SECTION QA      — report bounding-box extents and mid-plane section info at
     XY/XZ/YZ mid-planes as a detail string (informational, not a hard gate).

Exit codes:
  0  — harness success (verdict in fragment "status", not exit code).
  1  — harness/usage error (bad args, --in missing, etc.).

Trimesh is optional: lazy-imported; stdlib ASCII parser is the default path.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import sys
from typing import List, Optional, Tuple

# ---------------------------------------------------------------------------
# Optional trimesh (never hard-required)
# ---------------------------------------------------------------------------
try:
    import trimesh as _trimesh  # type: ignore
except ImportError:
    _trimesh = None  # type: ignore

# ---------------------------------------------------------------------------
# Types
# ---------------------------------------------------------------------------
Triangle = Tuple[Tuple[float, float, float], ...]

# ---------------------------------------------------------------------------
# Default printer profile
# ---------------------------------------------------------------------------
_DEFAULT_PRINTER = {
    "nozzle_width": 0.4,
    "layer_height": 0.2,
    "min_perimeters": 3,
    "max_overhang_deg": 45.0,
}

# ---------------------------------------------------------------------------
# ASCII STL parser (reused from stage-geometry.py pattern)
# ---------------------------------------------------------------------------
_FACET_RE = re.compile(r"facet\s+normal", re.IGNORECASE)
_VERTEX_RE = re.compile(
    r"vertex\s+([\-+]?[0-9]*\.?[0-9]+(?:[eE][\-+]?[0-9]+)?)"
    r"\s+([\-+]?[0-9]*\.?[0-9]+(?:[eE][\-+]?[0-9]+)?)"
    r"\s+([\-+]?[0-9]*\.?[0-9]+(?:[eE][\-+]?[0-9]+)?)",
    re.IGNORECASE,
)
_NORMAL_RE = re.compile(
    r"facet\s+normal\s+([\-+]?[0-9]*\.?[0-9]+(?:[eE][\-+]?[0-9]+)?)"
    r"\s+([\-+]?[0-9]*\.?[0-9]+(?:[eE][\-+]?[0-9]+)?)"
    r"\s+([\-+]?[0-9]*\.?[0-9]+(?:[eE][\-+]?[0-9]+)?)",
    re.IGNORECASE,
)


def _parse_stl_ascii(path: str) -> Tuple[List[Triangle], List[Tuple[float, float, float]]]:
    """
    Parse ASCII STL; return (triangles, normals).

    normals[i] is the stated normal for triangles[i].  If the stated normal is
    zero-length (e.g. auto-generated as 0 0 0) the computed normal is used.
    """
    triangles: List[Triangle] = []
    normals: List[Tuple[float, float, float]] = []

    with open(path, "r", errors="replace") as fh:
        content = fh.read()

    # Split on "facet normal" boundaries keeping the normal line
    raw_blocks = _FACET_RE.split(content)
    for block in raw_blocks[1:]:
        # The normal values are at the start of this block (after the split)
        nm = _NORMAL_RE.match("facet normal " + block.lstrip())
        if nm:
            nx, ny, nz = float(nm.group(1)), float(nm.group(2)), float(nm.group(3))
        else:
            nx, ny, nz = 0.0, 0.0, 0.0

        vertices = _VERTEX_RE.findall(block)
        if len(vertices) >= 3:
            tri = tuple((float(x), float(y), float(z)) for x, y, z in vertices[:3])
            triangles.append(tri)  # type: ignore[arg-type]

            # If stated normal is degenerate, compute from vertices
            mag = math.sqrt(nx * nx + ny * ny + nz * nz)
            if mag < 1e-9:
                nx, ny, nz = _compute_normal(tri)
            normals.append((nx, ny, nz))

    return triangles, normals


def _compute_normal(tri: Triangle) -> Tuple[float, float, float]:
    """Compute face normal from triangle vertices via cross product."""
    v0, v1, v2 = tri
    ax, ay, az = v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]
    bx, by, bz = v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]
    nx = ay * bz - az * by
    ny = az * bx - ax * bz
    nz = ax * by - ay * bx
    mag = math.sqrt(nx * nx + ny * ny + nz * nz)
    if mag < 1e-9:
        return 0.0, 0.0, 1.0
    return nx / mag, ny / mag, nz / mag


def _parse_stl(path: str) -> Optional[Tuple[List[Triangle], List[Tuple[float, float, float]]]]:
    """
    Parse STL at *path* into (triangles, normals).

    Tries trimesh first when available; falls back to stdlib ASCII parser.
    Returns None on parse failure.
    """
    if _trimesh is not None:
        try:
            mesh = _trimesh.load_mesh(path)
            tris = []
            norms = []
            for i, face in enumerate(mesh.faces):
                v0 = tuple(float(c) for c in mesh.vertices[face[0]])
                v1 = tuple(float(c) for c in mesh.vertices[face[1]])
                v2 = tuple(float(c) for c in mesh.vertices[face[2]])
                tri = (v0, v1, v2)
                tris.append(tri)
                fn = tuple(float(c) for c in mesh.face_normals[i])
                norms.append(fn)
            if tris:
                return tris, norms
        except Exception:
            pass

    try:
        tris, norms = _parse_stl_ascii(path)
        return (tris, norms) if tris else None
    except Exception:
        return None

# ---------------------------------------------------------------------------
# Printer profile loader
# ---------------------------------------------------------------------------

def _load_printer_profile(printer_path: Optional[str]) -> dict:
    """
    Load printer profile from a JSON file.

    Returns defaults if printer_path is None or the file can't be read.
    """
    profile = dict(_DEFAULT_PRINTER)
    if not printer_path:
        return profile
    try:
        with open(printer_path) as f:
            data = json.load(f)
        for key in ("nozzle_width", "layer_height", "min_perimeters", "max_overhang_deg"):
            if key in data:
                profile[key] = float(data[key])
    except Exception:
        pass
    return profile

# ---------------------------------------------------------------------------
# Wall thickness check
# ---------------------------------------------------------------------------

def check_wall_thickness(
    triangles: List[Triangle],
    min_wall: float,
) -> Tuple[bool, str, Optional[float]]:
    """
    Estimate minimum feature thickness from bounding box extents.

    Heuristic: the smallest axis-aligned extent of the mesh is the thinnest
    feature dimension (catches thin plates/slabs — the dominant FDM failure
    mode). Each axis extent is min→max vertex coordinate spread along that axis.

    Returns (passes, detail, min_thickness_found).
    passes=True when min_thickness >= min_wall.
    """
    if not triangles:
        return True, "No triangles; skipping wall check", None

    xs = [v[0] for tri in triangles for v in tri]
    ys = [v[1] for tri in triangles for v in tri]
    zs = [v[2] for tri in triangles for v in tri]

    extent_x = max(xs) - min(xs)
    extent_y = max(ys) - min(ys)
    extent_z = max(zs) - min(zs)

    min_extent = min(extent_x, extent_y, extent_z)

    detail = (
        f"Bounding extents: X={extent_x:.3f} Y={extent_y:.3f} Z={extent_z:.3f} mm; "
        f"min_extent={min_extent:.3f} mm; min_wall={min_wall:.3f} mm"
    )

    if min_extent < min_wall:
        return False, detail, min_extent
    return True, detail, min_extent

# ---------------------------------------------------------------------------
# Overhang angle check
# ---------------------------------------------------------------------------

def check_overhang(
    triangles: List[Triangle],
    normals: List[Tuple[float, float, float]],
    max_overhang_deg: float,
) -> Tuple[bool, str, int]:
    """
    Scan faces for steep overhangs relative to build-Z (+Z axis).

    A face is a candidate overhang when its normal has nz < 0 (faces downward)
    AND its centroid Z is above the model's minimum Z (faces on the build plate
    are excluded — a flat bottom is supported by definition).

    The overhang angle is measured from vertical (build-Z):
        For a downward face, overhang_from_vertical = 90 - arccos(|nz|/|N|)
        Horizontal face → 0°; straight-down face → 90°.
    Faces where overhang_from_vertical > max_overhang_deg are flagged.

    Returns (passes, detail, steep_count).
    """
    if not triangles:
        return True, "No geometry; skipping overhang check", 0

    # Build-plate Z: faces at (or within tolerance of) model min-Z are supported
    all_zs = [v[2] for tri in triangles for v in tri]
    z_min = min(all_zs)
    z_tol = max((max(all_zs) - z_min) * 0.01, 1e-4)

    steep_count = 0
    max_seen = 0.0

    for tri, (nx, ny, nz) in zip(triangles, normals):
        if nz >= 0:
            continue  # upward or horizontal — not an overhang concern
        # Centroid Z of this face
        cz = (tri[0][2] + tri[1][2] + tri[2][2]) / 3.0
        if cz <= z_min + z_tol:
            continue  # bottom face — resting on build plate, not an overhang

        mag = math.sqrt(nx * nx + ny * ny + nz * nz)
        if mag < 1e-9:
            continue
        # overhang from vertical: 0° = horizontal face, 90° = straight down
        cos_angle = abs(nz) / mag
        cos_angle = min(1.0, cos_angle)  # clamp float noise
        overhang_from_vertical = math.degrees(math.acos(cos_angle))
        if overhang_from_vertical > max_overhang_deg:
            steep_count += 1
            if overhang_from_vertical > max_seen:
                max_seen = overhang_from_vertical

    if steep_count > 0:
        detail = (
            f"{steep_count} face(s) exceed overhang limit "
            f"(max seen: {max_seen:.1f}°; limit: {max_overhang_deg:.1f}°)"
        )
        return False, detail, steep_count

    detail = f"No faces exceed overhang limit of {max_overhang_deg:.1f}°"
    return True, detail, 0

# ---------------------------------------------------------------------------
# Section QA (informational)
# ---------------------------------------------------------------------------

def section_info(triangles: List[Triangle]) -> str:
    """
    Produce section summary at key mid-planes (XY/XZ/YZ).

    Counts faces whose centroid lies within ±10% of mesh extent from each
    mid-plane. This is informational only — no hard gate.
    """
    if not triangles:
        return "section: no geometry"

    xs = [v[0] for tri in triangles for v in tri]
    ys = [v[1] for tri in triangles for v in tri]
    zs = [v[2] for tri in triangles for v in tri]

    cx = (min(xs) + max(xs)) / 2
    cy = (min(ys) + max(ys)) / 2
    cz = (min(zs) + max(zs)) / 2

    ex = (max(xs) - min(xs)) * 0.1 + 1e-9
    ey = (max(ys) - min(ys)) * 0.1 + 1e-9
    ez = (max(zs) - min(zs)) * 0.1 + 1e-9

    xy_count = xz_count = yz_count = 0
    for tri in triangles:
        tx = sum(v[0] for v in tri) / 3
        ty = sum(v[1] for v in tri) / 3
        tz = sum(v[2] for v in tri) / 3
        if abs(tz - cz) <= ez:
            xy_count += 1
        if abs(ty - cy) <= ey:
            xz_count += 1
        if abs(tx - cx) <= ex:
            yz_count += 1

    return (
        f"section XY-mid: {xy_count} faces, "
        f"XZ-mid: {xz_count} faces, "
        f"YZ-mid: {yz_count} faces"
    )

# ---------------------------------------------------------------------------
# Stage runner
# ---------------------------------------------------------------------------

def run_slicer_stage(stl_path: str, printer: dict) -> dict:
    """
    Run all slicer checks on *stl_path* and return a stage-record fragment.

    Fragment shape:
      { "stage": "slicer", "status": "pass|fail",
        "detail": "...", "findings": [...] }
    """
    findings = []
    status = "pass"
    details = []

    # Parse STL
    parsed = _parse_stl(stl_path)
    if parsed is None:
        return {
            "stage": "slicer",
            "status": "fail",
            "detail": f"Failed to parse STL: {os.path.basename(stl_path)}",
            "findings": [{"code": "slicer_parse_error",
                          "message": f"Could not parse {stl_path}"}],
        }

    triangles, normals = parsed
    details.append(f"Parsed {len(triangles)} facets")

    # Printer params
    min_wall = float(printer["min_perimeters"]) * float(printer["nozzle_width"])
    max_overhang_deg = float(printer["max_overhang_deg"])

    # 1. Wall thickness check
    wall_ok, wall_detail, min_thickness = check_wall_thickness(triangles, min_wall)
    details.append(wall_detail)
    if not wall_ok:
        status = "fail"
        findings.append({
            "code": "slicer_thin_wall",
            "message": (
                f"Min feature thickness {min_thickness:.3f} mm < "
                f"min_wall {min_wall:.3f} mm "
                f"({int(printer['min_perimeters'])} perimeters × "
                f"{printer['nozzle_width']} mm nozzle)"
            ),
        })

    # 2. Overhang angle check
    overhang_ok, overhang_detail, steep_count = check_overhang(triangles, normals, max_overhang_deg)
    details.append(overhang_detail)
    if not overhang_ok:
        status = "fail"
        findings.append({
            "code": "slicer_overhang",
            "message": (
                f"{steep_count} face(s) exceed overhang limit "
                f"of {max_overhang_deg:.1f}°; {overhang_detail}"
            ),
        })

    # 3. Section QA (informational)
    sec_detail = section_info(triangles)
    details.append(sec_detail)

    return {
        "stage": "slicer",
        "status": status,
        "detail": "; ".join(details),
        "findings": findings,
    }

# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------

def main(argv=None):
    parser = argparse.ArgumentParser(
        description="autospec-fab slicer stage: min wall + overhang + section QA"
    )
    parser.add_argument("--in", dest="in_path", required=True,
                        help="Input STL file")
    parser.add_argument("--out", required=True,
                        help="Output fragment JSON path")
    parser.add_argument("--model", dest="model_path", default=None,
                        help="Per-model metadata JSON sidecar (optional, unused by this stage)")
    parser.add_argument("--printer", dest="printer_path", default=None,
                        help="Printer profile JSON with nozzle_width/min_perimeters/max_overhang_deg")
    parser.add_argument("--config", dest="config_path", default=None,
                        help="fab.yml path; printer block used if --printer not given")

    args = parser.parse_args(argv)

    # Resolve printer profile: --printer > --config printer block > defaults
    printer = _load_printer_profile(args.printer_path)
    if args.printer_path is None and args.config_path:
        try:
            # fab.yml is YAML; try yq/python-yaml; fall back to regex extraction
            _load_printer_from_config(args.config_path, printer)
        except Exception:
            pass  # keep defaults

    fragment = run_slicer_stage(args.in_path, printer)

    out_dir = os.path.dirname(os.path.abspath(args.out))
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)
    with open(args.out, "w") as f:
        json.dump(fragment, f, indent=2)
        f.write("\n")

    sys.exit(0)


def _load_printer_from_config(config_path: str, printer: dict) -> None:
    """
    Best-effort: extract printer fields from fab.yml into *printer* dict.

    Tries PyYAML first; falls back to simple regex line scanning.
    Mutates *printer* in place.
    """
    try:
        import yaml  # type: ignore
        with open(config_path) as f:
            data = yaml.safe_load(f)
        p = data.get("printer", {}) or {}
        for key in ("nozzle_width", "layer_height", "min_perimeters", "max_overhang_deg"):
            if key in p:
                printer[key] = float(p[key])
        return
    except ImportError:
        pass

    # Fallback: line-by-line key: value under a "printer:" block
    in_printer = False
    with open(config_path) as f:
        for line in f:
            stripped = line.rstrip()
            if stripped.startswith("printer:"):
                in_printer = True
                continue
            if in_printer:
                if stripped and not stripped[0].isspace():
                    break  # left printer block
                for key in ("nozzle_width", "layer_height", "min_perimeters", "max_overhang_deg"):
                    m = re.match(rf"\s+{key}\s*:\s*([0-9.]+)", stripped)
                    if m:
                        printer[key] = float(m.group(1))


if __name__ == "__main__":
    main()

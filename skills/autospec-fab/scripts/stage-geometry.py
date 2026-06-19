#!/usr/bin/env python3
"""
stage-geometry.py — geometry gate stage for autospec-fab.

Uniform stage CLI:
    stage-geometry.py --in <stl|dir> --out <fragment.json>
                      [--model <metadata.json>] [--expect-bodies N]

Checks performed:
  1. RELOAD   — file opens and parses to ≥1 facet.
  2. WATERTIGHT — every undirected edge shared by exactly 2 facets.
  3. SINGLE-BODY — connected components over facet-adjacency graph == expected.

Exit codes:
  0  — harness success (verdict lives in fragment "status", not exit code).
  1  — harness/usage error (bad args, --in missing, etc.).

Trimesh is optional: lazy-imported and used only when available (for
robustness / binary STL support). The stdlib ASCII parser is the default
path and the one exercised by tests.
"""

from __future__ import annotations

import argparse
import json
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

# A triangle is a tuple of three (x, y, z) float tuples.
Triangle = Tuple[Tuple[float, float, float], ...]


# ---------------------------------------------------------------------------
# ASCII STL parser (stdlib)
# ---------------------------------------------------------------------------

_FACET_RE = re.compile(r"facet\s+normal", re.IGNORECASE)
_VERTEX_RE = re.compile(
    r"vertex\s+([\-+]?[0-9]*\.?[0-9]+(?:[eE][\-+]?[0-9]+)?)"
    r"\s+([\-+]?[0-9]*\.?[0-9]+(?:[eE][\-+]?[0-9]+)?)"
    r"\s+([\-+]?[0-9]*\.?[0-9]+(?:[eE][\-+]?[0-9]+)?)",
    re.IGNORECASE,
)


def _parse_stl_ascii(path: str) -> List[Triangle]:
    """Parse ASCII STL and return list of triangles (each: 3 vertices)."""
    triangles: List[Triangle] = []
    with open(path, "r", errors="replace") as fh:
        content = fh.read()

    # Split on facet boundaries
    facet_blocks = _FACET_RE.split(content)
    for block in facet_blocks[1:]:  # skip header before first facet
        vertices = _VERTEX_RE.findall(block)
        if len(vertices) >= 3:
            tri = tuple(
                (float(x), float(y), float(z)) for x, y, z in vertices[:3]
            )
            triangles.append(tri)  # type: ignore[arg-type]
    return triangles


def _parse_stl(path: str) -> Optional[List[Triangle]]:
    """
    Parse STL at *path* into a list of triangles.

    Tries trimesh first (handles binary STL too) when available; falls back to
    the stdlib ASCII parser.  Returns None on parse failure.
    """
    if _trimesh is not None:
        try:
            mesh = _trimesh.load_mesh(path)
            tris = []
            for face in mesh.faces:
                v0 = tuple(float(c) for c in mesh.vertices[face[0]])
                v1 = tuple(float(c) for c in mesh.vertices[face[1]])
                v2 = tuple(float(c) for c in mesh.vertices[face[2]])
                tris.append((v0, v1, v2))
            return tris if tris else None
        except Exception:
            pass  # fall through to stdlib parser

    try:
        tris = _parse_stl_ascii(path)
        return tris if tris else None
    except Exception:
        return None


# ---------------------------------------------------------------------------
# Geometry checks
# ---------------------------------------------------------------------------

_TOLERANCE = 1e-6


def _rounded_vertex(v: tuple, tol: float = _TOLERANCE) -> tuple:
    """Round vertex coordinates to *tol* to deduplicate float noise."""
    scale = 1.0 / tol
    return (round(v[0] * scale), round(v[1] * scale), round(v[2] * scale))


def _edge_key(va: tuple, vb: tuple) -> tuple:
    """Return a canonical (sorted) undirected edge key."""
    a = _rounded_vertex(va)
    b = _rounded_vertex(vb)
    return (a, b) if a <= b else (b, a)


def check_watertight(triangles: List[Triangle]) -> Tuple[bool, str]:
    """
    Check watertightness: every undirected edge must be shared by exactly 2 facets.

    Returns (is_watertight, detail_string).
    """
    from collections import defaultdict

    edge_count: dict = defaultdict(int)
    for tri in triangles:
        v0, v1, v2 = tri
        edge_count[_edge_key(v0, v1)] += 1
        edge_count[_edge_key(v1, v2)] += 1
        edge_count[_edge_key(v2, v0)] += 1

    bad_edges = [e for e, c in edge_count.items() if c != 2]
    if bad_edges:
        return False, (
            f"Non-watertight: {len(bad_edges)} edge(s) not shared by exactly "
            f"2 facets (open/non-manifold mesh)"
        )
    return True, "All edges shared by exactly 2 facets (watertight)"


def check_connectivity(triangles: List[Triangle]) -> Tuple[int, str]:
    """
    Count connected components via facet-adjacency (facets sharing an edge).

    Returns (component_count, detail_string).
    """
    from collections import defaultdict, deque

    # Build edge → list-of-facet-indices map
    edge_to_facets: dict = defaultdict(list)
    for i, tri in enumerate(triangles):
        v0, v1, v2 = tri
        edge_to_facets[_edge_key(v0, v1)].append(i)
        edge_to_facets[_edge_key(v1, v2)].append(i)
        edge_to_facets[_edge_key(v2, v0)].append(i)

    # Build adjacency list: facet i → set of neighbouring facet indices
    n = len(triangles)
    adj: List[set] = [set() for _ in range(n)]
    for facets in edge_to_facets.values():
        for a in facets:
            for b in facets:
                if a != b:
                    adj[a].add(b)

    # BFS to count components
    visited = [False] * n
    components = 0
    for start in range(n):
        if visited[start]:
            continue
        components += 1
        queue: deque = deque([start])
        while queue:
            node = queue.popleft()
            if visited[node]:
                continue
            visited[node] = True
            for nb in adj[node]:
                if not visited[nb]:
                    queue.append(nb)

    return components, f"{components} connected body/bodies found"


# ---------------------------------------------------------------------------
# Stage runner
# ---------------------------------------------------------------------------

def run_geometry_stage(
    stl_path: str,
    expect_bodies: int = 1,
) -> dict:
    """
    Run all geometry checks on *stl_path* and return a stage-record fragment.

    Fragment shape:
      { "stage": "geometry", "status": "pass|fail",
        "detail": "...", "findings": [...] }
    """
    findings = []
    status = "pass"
    details = []

    # ------------------------------------------------------------------
    # 1. RELOAD check
    # ------------------------------------------------------------------
    triangles = _parse_stl(stl_path)
    if triangles is None or len(triangles) == 0:
        findings.append({
            "code": "geometry_reload",
            "message": f"Failed to parse STL or zero facets: {stl_path}",
        })
        return {
            "stage": "geometry",
            "status": "fail",
            "detail": f"RELOAD failed: could not parse {os.path.basename(stl_path)}",
            "findings": findings,
        }

    details.append(f"Parsed {len(triangles)} facets")

    # ------------------------------------------------------------------
    # 2. WATERTIGHT check
    # ------------------------------------------------------------------
    is_wt, wt_detail = check_watertight(triangles)
    details.append(wt_detail)
    if not is_wt:
        status = "fail"
        findings.append({
            "code": "non_watertight",
            "message": wt_detail,
        })

    # ------------------------------------------------------------------
    # 3. SINGLE-BODY connectivity check
    # ------------------------------------------------------------------
    body_count, conn_detail = check_connectivity(triangles)
    details.append(conn_detail)
    if body_count != expect_bodies:
        status = "fail"
        findings.append({
            "code": "disconnected_bodies",
            "message": (
                f"Expected {expect_bodies} body/bodies, "
                f"found {body_count}: {conn_detail}"
            ),
        })

    return {
        "stage": "geometry",
        "status": status,
        "detail": "; ".join(details),
        "findings": findings,
    }


def _collect_stl_paths(in_arg: str) -> List[str]:
    """Return list of STL file paths from *in_arg* (file or directory)."""
    if os.path.isdir(in_arg):
        paths = []
        for name in sorted(os.listdir(in_arg)):
            if name.lower().endswith(".stl"):
                paths.append(os.path.join(in_arg, name))
        return paths
    return [in_arg]


# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------

def main(argv=None):
    parser = argparse.ArgumentParser(
        description="autospec-fab geometry stage: reload + watertight + connectivity"
    )
    parser.add_argument("--in", dest="in_path", required=True,
                        help="Input STL file or directory of STL files")
    parser.add_argument("--out", required=True,
                        help="Output fragment JSON path")
    parser.add_argument("--model", dest="model_path", default=None,
                        help="Per-model metadata JSON sidecar (optional)")
    parser.add_argument("--expect-bodies", dest="expect_bodies",
                        type=int, default=None,
                        help="Expected number of connected bodies (default: 1, "
                             "or from metadata body_count if provided)")

    args = parser.parse_args(argv)

    # Resolve expect_bodies: CLI arg > metadata > default 1
    expect_bodies = args.expect_bodies
    if expect_bodies is None and args.model_path:
        try:
            with open(args.model_path) as f:
                meta = json.load(f)
            expect_bodies = int(meta.get("body_count", 1))
        except Exception:
            expect_bodies = 1
    if expect_bodies is None:
        expect_bodies = 1

    stl_paths = _collect_stl_paths(args.in_path)
    if not stl_paths:
        print(f"stage-geometry: no STL files found at {args.in_path!r}",
              file=sys.stderr)
        sys.exit(1)

    # Run checks on first STL (directory mode: first file only for now,
    # matching single-model stage contract; engine iterates per-model).
    fragment = run_geometry_stage(stl_paths[0], expect_bodies=expect_bodies)

    # Write fragment
    out_dir = os.path.dirname(os.path.abspath(args.out))
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)
    with open(args.out, "w") as f:
        json.dump(fragment, f, indent=2)
        f.write("\n")

    # Exit 0 always on harness success (verdict in status field)
    sys.exit(0)


if __name__ == "__main__":
    main()

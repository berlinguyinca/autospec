#!/usr/bin/env python3
"""
render_views.py — the deterministic required-view table for autospec-fab render QA.

The fab design spec (§Release-gate stage 12, "render QA + 16-view contact sheet")
requires every printable model be rendered from a fixed, named set of >=16 view
angles so the vision stage (#1232) and human reviewers can inspect every port,
socket, gasket, lug, and overhang:

  - the 6 orthographic axis views (front/back/left/right/top/bottom),
  - the 8 corner (45 degree) diagonals exposing edges/ports between two faces,
  - >=2 slicer-like / print-orientation rotated views.

The table is a fixed ordered list (NOT a set) so both the count and the names
are deterministic and testable. Each entry carries a FreeCAD camera dict
(camera/azimuth/elevation/width/height) consumed by freecad_harness.render();
the real camera math is applied inside FreeCAD (mocked in tests), so unknown
camera names degrade to an isometric view rather than failing.
"""

_W = 800
_H = 600


def _v(name, kind, camera, azimuth=0.0, elevation=0.0):
    """Build one view-table entry (name + classification + camera dict)."""
    return {
        "name": name,
        "kind": kind,
        "camera": {
            "camera": camera,
            "azimuth": float(azimuth),
            "elevation": float(elevation),
            "width": _W,
            "height": _H,
        },
    }


# 6 orthographic axis views.
_AXIS = [
    _v("front", "axis", "front", 0, 0),
    _v("back", "axis", "back", 180, 0),
    _v("left", "axis", "left", 90, 0),
    _v("right", "axis", "right", -90, 0),
    _v("top", "axis", "top", 0, 90),
    _v("bottom", "axis", "bottom", 0, -90),
]

# 8 corner (45 degree) diagonals: the four upper and four lower corners.
_DIAGONAL = [
    _v("iso-front-right-top", "diagonal", "iso", 45, 35),
    _v("iso-back-right-top", "diagonal", "iso", 135, 35),
    _v("iso-back-left-top", "diagonal", "iso", 225, 35),
    _v("iso-front-left-top", "diagonal", "iso", 315, 35),
    _v("iso-front-right-bottom", "diagonal", "iso", 45, -35),
    _v("iso-back-right-bottom", "diagonal", "iso", 135, -35),
    _v("iso-back-left-bottom", "diagonal", "iso", 225, -35),
    _v("iso-front-left-bottom", "diagonal", "iso", 315, -35),
]

# Slicer-like rotated / print-orientation views: looking at the part as it sits
# on the build plate plus a tipped orientation that exposes overhang undersides.
_SLICER = [
    _v("slicer-build-plate-top", "slicer", "top", 0, 90),
    _v("slicer-rotated-45", "slicer", "iso", 30, 20),
    _v("slicer-overhang-underside", "slicer", "iso", 0, -60),
]

# The authoritative ordered table. Order is load-bearing (contact-sheet layout).
REQUIRED_VIEWS = _AXIS + _DIAGONAL + _SLICER

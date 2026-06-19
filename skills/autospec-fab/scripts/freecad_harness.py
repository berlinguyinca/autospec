"""
freecad_harness.py — headless FreeCAD driver (stdlib only).

Drives freecadcmd via subprocess; never imports FreeCAD or trimesh at module
level so the file is safely importable on any Python 3 host.

Public API
----------
resolve_freecadcmd()          -> str | None
open_model(path)              -> subprocess.CompletedProcess
export_stl(in_path, out_stl)  -> subprocess.CompletedProcess
section(in_path, plane, out_path) -> subprocess.CompletedProcess
render(in_path, view, out_png)    -> subprocess.CompletedProcess
"""

import os
import shutil
import subprocess
import tempfile
import textwrap


# ---------------------------------------------------------------------------
# Resolution
# ---------------------------------------------------------------------------

def resolve_freecadcmd():
    """Return the absolute path to freecadcmd, or None if not on PATH."""
    return shutil.which("freecadcmd")


def _require_freecadcmd():
    """Return freecadcmd path or raise FileNotFoundError."""
    cmd = resolve_freecadcmd()
    if cmd is None:
        raise FileNotFoundError(
            "freecadcmd not found on PATH. "
            "Install FreeCAD or prepend a $TMP/bin shim to $PATH."
        )
    return cmd


# ---------------------------------------------------------------------------
# Script builders — deterministic, no timestamps/random tokens
# ---------------------------------------------------------------------------

def _script_open(in_path):
    """FreeCAD Python script: open a model and report its label."""
    in_path = os.path.abspath(in_path)
    return textwrap.dedent(f"""\
        import FreeCAD
        FreeCAD.units.setSchema(FreeCAD.Units.Millimeter)
        doc = FreeCAD.open({in_path!r})
        print("opened:", doc.Label)
        FreeCAD.closeDocument(doc.Label)
    """)


def _script_export_stl(in_path, out_stl):
    """FreeCAD Python script: export model to STL (mm, ASCII)."""
    in_path = os.path.abspath(in_path)
    out_stl = os.path.abspath(out_stl)
    return textwrap.dedent(f"""\
        import FreeCAD
        import Mesh
        FreeCAD.units.setSchema(FreeCAD.Units.Millimeter)
        doc = FreeCAD.open({in_path!r})
        objects = doc.Objects
        if not objects:
            raise RuntimeError("no objects in document: " + {in_path!r})
        Mesh.export(objects, {out_stl!r})
        print("exported:", {out_stl!r})
        FreeCAD.closeDocument(doc.Label)
    """)


def _script_section(in_path, plane, out_path):
    """FreeCAD Python script: cut a cross-section and export as DXF/SVG."""
    in_path = os.path.abspath(in_path)
    out_path = os.path.abspath(out_path)
    # plane is a dict: {"normal": [nx,ny,nz], "origin": [ox,oy,oz]}
    nx, ny, nz = plane.get("normal", [0, 0, 1])
    ox, oy, oz = plane.get("origin", [0, 0, 0])
    return textwrap.dedent(f"""\
        import FreeCAD
        import Part
        FreeCAD.units.setSchema(FreeCAD.Units.Millimeter)
        doc = FreeCAD.open({in_path!r})
        shape = Part.makeCompound([o.Shape for o in doc.Objects
                                   if hasattr(o, 'Shape')])
        normal = FreeCAD.Vector({nx}, {ny}, {nz})
        origin = FreeCAD.Vector({ox}, {oy}, {oz})
        section = shape.section(Part.Plane(origin, normal))
        section.exportDXF({out_path!r})
        print("section:", {out_path!r})
        FreeCAD.closeDocument(doc.Label)
    """)


def _script_render(in_path, view, out_png):
    """FreeCAD Python script: headless render to PNG (seed-free, pinned args)."""
    in_path = os.path.abspath(in_path)
    out_png = os.path.abspath(out_png)
    # view is a dict: {"camera": "iso"|"top"|"front"|"right", "width": int, "height": int}
    camera = view.get("camera", "iso")
    width = int(view.get("width", 800))
    height = int(view.get("height", 600))
    return textwrap.dedent(f"""\
        import FreeCAD
        import FreeCADGui
        FreeCAD.units.setSchema(FreeCAD.Units.Millimeter)
        FreeCADGui.showMainWindow()
        doc = FreeCAD.open({in_path!r})
        view = FreeCADGui.ActiveDocument.ActiveView
        view.setAnimationEnabled(False)
        camera = {camera!r}
        if camera == "iso":
            view.viewIsometric()
        elif camera == "top":
            view.viewTop()
        elif camera == "front":
            view.viewFront()
        elif camera == "right":
            view.viewRight()
        view.fitAll()
        view.saveImage({out_png!r}, {width}, {height}, "White")
        print("rendered:", {out_png!r})
        FreeCAD.closeDocument(doc.Label)
    """)


# ---------------------------------------------------------------------------
# Runners
# ---------------------------------------------------------------------------

def _run_script(script_text, extra_args=None):
    """Write script to a temp file, run freecadcmd, return CompletedProcess."""
    cmd = resolve_freecadcmd()
    if cmd is None:
        # Return a sentinel CompletedProcess so callers get a clear error
        # without crashing; stdout/stderr carry the diagnostic.
        return subprocess.CompletedProcess(
            args=["freecadcmd"],
            returncode=127,
            stdout="",
            stderr="freecadcmd not found on PATH",
        )
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".py", prefix="freecad_harness_", delete=False
    ) as tf:
        tf.write(script_text)
        script_path = tf.name
    try:
        args = [cmd, script_path]
        if extra_args:
            args.extend(extra_args)
        result = subprocess.run(
            args,
            capture_output=True,
            text=True,
        )
    finally:
        os.unlink(script_path)
    return result


def open_model(path):
    """Open a FreeCAD model and return the subprocess result."""
    return _run_script(_script_open(path))


def export_stl(in_path, out_stl):
    """Export a FreeCAD model to STL and return the subprocess result."""
    return _run_script(_script_export_stl(in_path, out_stl))


def section(in_path, plane, out_path):
    """Cut a cross-section from a model and return the subprocess result.

    plane: dict with keys "normal" ([nx,ny,nz]) and "origin" ([ox,oy,oz]).
    out_path: destination .dxf file.
    """
    return _run_script(_script_section(in_path, plane, out_path))


def render(in_path, view, out_png):
    """Render a model to PNG and return the subprocess result.

    view: dict with keys "camera" (iso/top/front/right), "width", "height".
    """
    return _run_script(_script_render(in_path, view, out_png))

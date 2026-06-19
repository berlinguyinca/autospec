"""
test_freecad_harness.py — unittest for freecad_harness.py.

Strategy: prepend a $TMP/bin directory containing a fake `freecadcmd`
Python shim to PATH.  The shim (freecad_shim.py, installed as `freecadcmd`):
  - Records its argv to a file (env var FREECAD_ARGV_RECORD).
  - Reads the FreeCAD Python script it was given and parses output paths
    from Mesh.export / exportDXF / saveImage calls.
  - Writes minimal sentinel content to each output path.
  - Exits 0.

No real FreeCAD is needed.  Pure stdlib.
"""

import os
import shutil
import stat
import sys
import tempfile
import unittest

# ---------------------------------------------------------------------------
# Ensure freecad_harness is importable regardless of invocation style.
# ---------------------------------------------------------------------------
_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_SCRIPTS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "scripts"))
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

import freecad_harness  # noqa: E402

# Path to the shim script that lives next to this test file.
_SHIM_SRC = os.path.join(_THIS_DIR, "freecad_shim.py")


# ---------------------------------------------------------------------------
# Shim installer
# ---------------------------------------------------------------------------

def _install_shim(tmp_bin, argv_record):
    """Copy freecad_shim.py into tmp_bin as 'freecadcmd' and make it executable."""
    shim_dst = os.path.join(tmp_bin, "freecadcmd")
    shutil.copy2(_SHIM_SRC, shim_dst)
    current = os.stat(shim_dst).st_mode
    os.chmod(shim_dst, current | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return shim_dst


# ---------------------------------------------------------------------------
# Base class: shim PATH setup/teardown
# ---------------------------------------------------------------------------

class _HarnessBase(unittest.TestCase):

    def setUp(self):
        self._orig_path = os.environ.get("PATH", "")
        self._tmpdir = tempfile.mkdtemp(prefix="fc_test_")
        self._tmp_bin = os.path.join(self._tmpdir, "bin")
        os.makedirs(self._tmp_bin)
        self._argv_record = os.path.join(self._tmpdir, "argv.txt")
        _install_shim(self._tmp_bin, self._argv_record)
        os.environ["PATH"] = self._tmp_bin + os.pathsep + self._orig_path
        os.environ["FREECAD_ARGV_RECORD"] = self._argv_record

    def tearDown(self):
        os.environ["PATH"] = self._orig_path
        os.environ.pop("FREECAD_ARGV_RECORD", None)
        shutil.rmtree(self._tmpdir, ignore_errors=True)

    def _recorded_lines(self):
        if not os.path.exists(self._argv_record):
            return []
        with open(self._argv_record) as f:
            return [ln.rstrip() for ln in f if ln.strip()]


# ---------------------------------------------------------------------------
# resolve_freecadcmd
# ---------------------------------------------------------------------------

class TestResolveFreecadcmd(_HarnessBase):

    def test_finds_shim_on_path(self):
        result = freecad_harness.resolve_freecadcmd()
        self.assertIsNotNone(result)
        self.assertIn("freecadcmd", result)

    def test_returns_none_when_absent(self):
        os.environ["PATH"] = "/usr/bin:/bin"
        result = freecad_harness.resolve_freecadcmd()
        self.assertIsNone(result)


# ---------------------------------------------------------------------------
# open_model
# ---------------------------------------------------------------------------

class TestOpenModel(_HarnessBase):

    def test_invokes_freecadcmd(self):
        result = freecad_harness.open_model("/fake/model.FCStd")
        self.assertEqual(result.returncode, 0,
                         f"stderr: {result.stderr}")

    def test_argv_record_has_entry(self):
        freecad_harness.open_model("/fake/model.FCStd")
        self.assertGreater(len(self._recorded_lines()), 0, "No argv recorded")

    def test_script_contains_freecad_open(self):
        script = freecad_harness._script_open("/model.FCStd")
        self.assertIn("FreeCAD.open", script)

    def test_script_pins_mm_units(self):
        script = freecad_harness._script_open("/model.FCStd")
        self.assertIn("Millimeter", script)

    def test_script_no_random_tokens(self):
        script = freecad_harness._script_open("/model.FCStd")
        self.assertNotIn("import random", script)
        self.assertNotIn("import time", script)
        self.assertNotIn("uuid", script)
        self.assertNotIn("strftime", script)


# ---------------------------------------------------------------------------
# export_stl
# ---------------------------------------------------------------------------

class TestExportSTL(_HarnessBase):

    def test_invokes_freecadcmd(self):
        out = os.path.join(self._tmpdir, "out.stl")
        result = freecad_harness.export_stl("/fake/model.FCStd", out)
        self.assertEqual(result.returncode, 0,
                         f"stderr: {result.stderr}")

    def test_produces_stl_at_requested_path(self):
        out = os.path.join(self._tmpdir, "output.stl")
        freecad_harness.export_stl("/fake/model.FCStd", out)
        self.assertTrue(os.path.exists(out),
                        f"Expected STL at {out}")

    def test_stl_output_starts_with_solid(self):
        out = os.path.join(self._tmpdir, "output.stl")
        freecad_harness.export_stl("/fake/model.FCStd", out)
        with open(out) as f:
            first = f.read(20)
        self.assertTrue(first.strip().startswith("solid"), repr(first))

    def test_script_calls_mesh_export_with_output_path(self):
        out_stl = "/my/output.stl"
        script = freecad_harness._script_export_stl("/in.FCStd", out_stl)
        self.assertIn("Mesh.export", script)
        self.assertIn(out_stl, script)

    def test_script_pins_mm_units(self):
        script = freecad_harness._script_export_stl("/in.FCStd", "/out.stl")
        self.assertIn("Millimeter", script)

    def test_script_no_random_tokens(self):
        script = freecad_harness._script_export_stl("/in.FCStd", "/out.stl")
        self.assertNotIn("import random", script)
        self.assertNotIn("import time", script)
        self.assertNotIn("uuid", script)
        self.assertNotIn("strftime", script)

    def test_script_deterministic_for_same_inputs(self):
        s1 = freecad_harness._script_export_stl("/in.FCStd", "/out.stl")
        s2 = freecad_harness._script_export_stl("/in.FCStd", "/out.stl")
        self.assertEqual(s1, s2)


# ---------------------------------------------------------------------------
# section
# ---------------------------------------------------------------------------

class TestSection(_HarnessBase):

    def test_invokes_freecadcmd(self):
        out = os.path.join(self._tmpdir, "section.dxf")
        plane = {"normal": [0, 0, 1], "origin": [0, 0, 10]}
        result = freecad_harness.section("/fake/model.FCStd", plane, out)
        self.assertEqual(result.returncode, 0,
                         f"stderr: {result.stderr}")

    def test_script_contains_section_call(self):
        plane = {"normal": [0, 0, 1], "origin": [0, 0, 10]}
        script = freecad_harness._script_section("/in.FCStd", plane, "/out.dxf")
        self.assertIn("shape.section", script)

    def test_script_pins_mm_units(self):
        plane = {"normal": [0, 0, 1], "origin": [0, 0, 0]}
        script = freecad_harness._script_section("/in.FCStd", plane, "/out.dxf")
        self.assertIn("Millimeter", script)

    def test_script_embeds_plane_values(self):
        plane = {"normal": [1, 0, 0], "origin": [5, 5, 5]}
        script = freecad_harness._script_section("/in.FCStd", plane, "/out.dxf")
        self.assertIn("1, 0, 0", script)
        self.assertIn("5, 5, 5", script)

    def test_script_no_random_tokens(self):
        plane = {"normal": [0, 0, 1], "origin": [0, 0, 0]}
        script = freecad_harness._script_section("/in.FCStd", plane, "/out.dxf")
        self.assertNotIn("import random", script)
        self.assertNotIn("import time", script)
        self.assertNotIn("uuid", script)


# ---------------------------------------------------------------------------
# render
# ---------------------------------------------------------------------------

class TestRender(_HarnessBase):

    def test_invokes_freecadcmd(self):
        out = os.path.join(self._tmpdir, "render.png")
        view = {"camera": "iso", "width": 800, "height": 600}
        result = freecad_harness.render("/fake/model.FCStd", view, out)
        self.assertEqual(result.returncode, 0,
                         f"stderr: {result.stderr}")

    def test_script_pins_camera_top(self):
        view = {"camera": "top", "width": 1024, "height": 768}
        script = freecad_harness._script_render("/in.FCStd", view, "/out.png")
        self.assertIn("viewTop", script)
        self.assertIn("1024", script)
        self.assertIn("768", script)

    def test_script_pins_camera_iso(self):
        view = {"camera": "iso", "width": 800, "height": 600}
        script = freecad_harness._script_render("/in.FCStd", view, "/out.png")
        self.assertIn("viewIsometric", script)

    def test_script_pins_mm_units(self):
        view = {"camera": "iso", "width": 800, "height": 600}
        script = freecad_harness._script_render("/in.FCStd", view, "/out.png")
        self.assertIn("Millimeter", script)

    def test_script_disables_animation(self):
        view = {"camera": "iso", "width": 800, "height": 600}
        script = freecad_harness._script_render("/in.FCStd", view, "/out.png")
        self.assertIn("setAnimationEnabled(False)", script)

    def test_script_no_random_tokens(self):
        view = {"camera": "iso", "width": 800, "height": 600}
        script = freecad_harness._script_render("/in.FCStd", view, "/out.png")
        self.assertNotIn("import random", script)
        self.assertNotIn("import time", script)
        self.assertNotIn("uuid", script)
        self.assertNotIn("strftime", script)

    def test_script_deterministic(self):
        view = {"camera": "iso", "width": 800, "height": 600}
        s1 = freecad_harness._script_render("/in.FCStd", view, "/out.png")
        s2 = freecad_harness._script_render("/in.FCStd", view, "/out.png")
        self.assertEqual(s1, s2)


# ---------------------------------------------------------------------------
# absent freecadcmd — graceful degradation
# ---------------------------------------------------------------------------

class TestAbsentFreecadcmd(unittest.TestCase):

    def setUp(self):
        self._orig_path = os.environ.get("PATH", "")
        os.environ["PATH"] = "/usr/bin:/bin"

    def tearDown(self):
        os.environ["PATH"] = self._orig_path

    def test_resolve_returns_none(self):
        self.assertIsNone(freecad_harness.resolve_freecadcmd())

    def test_export_stl_returncode_127(self):
        result = freecad_harness.export_stl("/f.FCStd", "/f.stl")
        self.assertEqual(result.returncode, 127)
        self.assertIn("freecadcmd", result.stderr)

    def test_open_model_returncode_127(self):
        result = freecad_harness.open_model("/f.FCStd")
        self.assertEqual(result.returncode, 127)

    def test_section_returncode_127(self):
        result = freecad_harness.section(
            "/f.FCStd", {"normal": [0, 0, 1], "origin": [0, 0, 0]}, "/o.dxf"
        )
        self.assertEqual(result.returncode, 127)

    def test_render_returncode_127(self):
        result = freecad_harness.render(
            "/f.FCStd", {"camera": "iso", "width": 800, "height": 600}, "/o.png"
        )
        self.assertEqual(result.returncode, 127)


# ---------------------------------------------------------------------------
# box fixture integrity
# ---------------------------------------------------------------------------

class TestBoxFixture(unittest.TestCase):

    @property
    def _stl_path(self):
        here = os.path.dirname(os.path.abspath(__file__))
        return os.path.normpath(
            os.path.join(here, "..", "fixtures", "box", "box.stl")
        )

    def test_fixture_exists(self):
        self.assertTrue(os.path.exists(self._stl_path),
                        f"Missing: {self._stl_path}")

    def test_fixture_is_ascii_stl(self):
        with open(self._stl_path) as f:
            content = f.read()
        self.assertTrue(content.strip().startswith("solid"), content[:60])
        self.assertIn("endsolid", content)

    def test_fixture_has_12_triangles(self):
        with open(self._stl_path) as f:
            content = f.read()
        count = content.count("facet normal")
        self.assertEqual(count, 12,
                         f"Expected 12 facets (cube = 6 faces x 2), got {count}")

    def test_fixture_normals_are_unit_length(self):
        import re
        with open(self._stl_path) as f:
            content = f.read()
        normals = re.findall(
            r"facet normal\s+([\-\d.]+)\s+([\-\d.]+)\s+([\-\d.]+)", content
        )
        self.assertEqual(len(normals), 12)
        for nx, ny, nz in normals:
            mag2 = float(nx)**2 + float(ny)**2 + float(nz)**2
            self.assertAlmostEqual(mag2, 1.0, places=5,
                                   msg=f"Non-unit normal: {nx} {ny} {nz}")


if __name__ == "__main__":
    unittest.main()

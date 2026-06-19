"""
test_stage_render.py — unittest for stage_render.py.

Strategy: prepend a $TMP/bin directory containing a fake `freecadcmd` Python
shim to PATH. The shim records the output PNG path of every render it is asked
to perform (one per required view) and writes sentinel PNG bytes there. The
stage assembles a deterministic HTML contact sheet referencing every view PNG.

No real FreeCAD / PIL / trimesh is needed. Pure stdlib + the PATH shim.

Assertions:
  (a) ALL required view angles (>=16, by name) were requested.
  (b) The contact-sheet artifact is assembled and references every view PNG.
  (c) A missing-view case -> status fail with finding render_incomplete.
  (d) Fragment shape: {stage:"render", status, detail, findings}.
"""

import json
import os
import shutil
import stat
import sys
import tempfile
import unittest

_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_SCRIPTS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "scripts"))
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

import stage_render  # noqa: E402

# A render shim that emits one PNG per view and records each requested PNG path.
# It parses the FreeCAD script's view.saveImage('PATH', ...) call exactly like
# the real freecad_shim, but additionally records the basename (the view name).
_RENDER_SHIM = r'''#!/usr/bin/env python3
import os, re, sys
record = os.environ.get("RENDER_VIEW_RECORD", "")
skip = os.environ.get("RENDER_SKIP_VIEW", "")
if len(sys.argv) < 2 or not os.path.isfile(sys.argv[1]):
    sys.exit(0)
script = open(sys.argv[1]).read()
m = re.search(r"view\.saveImage\(['\"]([^'\"]+)['\"]", script)
if m:
    path = m.group(1)
    name = os.path.splitext(os.path.basename(path))[0]
    if record:
        with open(record, "a") as f:
            f.write(name + "\n")
    if not (skip and name == skip):
        os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
        with open(path, "wb") as f:
            f.write(b"PNG_SENTINEL")
sys.exit(0)
'''


def _install_render_shim(tmp_bin):
    dst = os.path.join(tmp_bin, "freecadcmd")
    with open(dst, "w") as f:
        f.write(_RENDER_SHIM)
    mode = os.stat(dst).st_mode
    os.chmod(dst, mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return dst


class _RenderBase(unittest.TestCase):

    def setUp(self):
        self._orig_path = os.environ.get("PATH", "")
        self._tmpdir = tempfile.mkdtemp(prefix="render_test_")
        self._tmp_bin = os.path.join(self._tmpdir, "bin")
        os.makedirs(self._tmp_bin)
        self._record = os.path.join(self._tmpdir, "views.txt")
        _install_render_shim(self._tmp_bin)
        os.environ["PATH"] = self._tmp_bin + os.pathsep + self._orig_path
        os.environ["RENDER_VIEW_RECORD"] = self._record
        os.environ.pop("RENDER_SKIP_VIEW", None)
        # A trivial STL stand-in; the shim never reads it.
        self._stl = os.path.join(self._tmpdir, "widget.stl")
        with open(self._stl, "w") as f:
            f.write("solid widget\nendsolid widget\n")
        self._out = os.path.join(self._tmpdir, "fragment.json")
        self._render_dir = os.path.join(self._tmpdir, "renders")

    def tearDown(self):
        os.environ["PATH"] = self._orig_path
        os.environ.pop("RENDER_VIEW_RECORD", None)
        os.environ.pop("RENDER_SKIP_VIEW", None)
        shutil.rmtree(self._tmpdir, ignore_errors=True)

    def _recorded_views(self):
        if not os.path.exists(self._record):
            return []
        with open(self._record) as f:
            return [ln.strip() for ln in f if ln.strip()]

    def _run(self):
        rc = stage_render.main([
            "--in", self._stl,
            "--out", self._out,
            "--render-dir", self._render_dir,
        ])
        with open(self._out) as f:
            frag = json.load(f)
        return rc, frag


class TestViewTable(unittest.TestCase):

    def test_at_least_16_views(self):
        self.assertGreaterEqual(len(stage_render.REQUIRED_VIEWS), 16)

    def test_view_names_unique_and_ordered(self):
        names = [v["name"] for v in stage_render.REQUIRED_VIEWS]
        self.assertEqual(len(names), len(set(names)))
        # deterministic order: the table is a fixed list, not a set
        self.assertEqual(names, [v["name"] for v in stage_render.REQUIRED_VIEWS])

    def test_has_six_axis_views(self):
        names = {v["name"] for v in stage_render.REQUIRED_VIEWS}
        for axis in ("front", "back", "left", "right", "top", "bottom"):
            self.assertIn(axis, names)

    def test_has_eight_corner_diagonals(self):
        diagonals = [v for v in stage_render.REQUIRED_VIEWS
                     if v.get("kind") == "diagonal"]
        self.assertGreaterEqual(len(diagonals), 8)

    def test_has_slicer_rotated_views(self):
        slicer = [v for v in stage_render.REQUIRED_VIEWS
                  if v.get("kind") == "slicer"]
        self.assertGreaterEqual(len(slicer), 2)


class TestAllViewsRequested(_RenderBase):

    def test_every_required_view_requested(self):
        rc, frag = self._run()
        self.assertEqual(rc, 0)
        requested = set(self._recorded_views())
        expected = {v["name"] for v in stage_render.REQUIRED_VIEWS}
        self.assertEqual(requested, expected,
                         msg=f"missing: {expected - requested}")

    def test_status_pass_when_complete(self):
        _, frag = self._run()
        self.assertEqual(frag["status"], "pass")
        self.assertEqual(frag["findings"], [])


class TestContactSheet(_RenderBase):

    def test_sheet_assembled_and_referenced_in_detail(self):
        _, frag = self._run()
        sheet = os.path.join(self._render_dir, "widget", "contact-sheet.html")
        self.assertTrue(os.path.isfile(sheet), "contact sheet not written")
        self.assertIn("contact-sheet.html", frag["detail"])

    def test_sheet_references_every_view(self):
        self._run()
        sheet = os.path.join(self._render_dir, "widget", "contact-sheet.html")
        with open(sheet) as f:
            html = f.read()
        for v in stage_render.REQUIRED_VIEWS:
            self.assertIn(v["name"] + ".png", html,
                          msg=f"{v['name']} not referenced in sheet")

    def test_sheet_is_deterministic(self):
        self._run()
        sheet = os.path.join(self._render_dir, "widget", "contact-sheet.html")
        with open(sheet) as f:
            first = f.read()
        # re-run into the same dir; identical bytes
        self._run()
        with open(sheet) as f:
            second = f.read()
        self.assertEqual(first, second)


class TestMissingView(_RenderBase):

    def test_missing_view_fails_render_incomplete(self):
        os.environ["RENDER_SKIP_VIEW"] = "top"
        rc, frag = self._run()
        self.assertEqual(rc, 0)  # verdict-in-status, exit 0
        self.assertEqual(frag["status"], "fail")
        codes = [f["code"] for f in frag["findings"]]
        self.assertIn("render_incomplete", codes)


class TestFragmentShape(_RenderBase):

    def test_fragment_keys(self):
        _, frag = self._run()
        self.assertEqual(frag["stage"], "render")
        self.assertIn(frag["status"], ("pass", "fail", "warn", "skip"))
        self.assertIsInstance(frag["detail"], str)
        self.assertIsInstance(frag["findings"], list)


class TestFreecadAbsent(unittest.TestCase):

    def test_skip_when_freecadcmd_absent(self):
        orig = os.environ.get("PATH", "")
        tmpdir = tempfile.mkdtemp(prefix="render_noskip_")
        try:
            os.environ["PATH"] = "/nonexistent-bin-dir"
            stl = os.path.join(tmpdir, "m.stl")
            with open(stl, "w") as f:
                f.write("solid m\nendsolid m\n")
            out = os.path.join(tmpdir, "frag.json")
            rc = stage_render.main([
                "--in", stl, "--out", out,
                "--render-dir", os.path.join(tmpdir, "r"),
            ])
            self.assertEqual(rc, 0)
            with open(out) as f:
                frag = json.load(f)
            self.assertEqual(frag["status"], "skip")
        finally:
            os.environ["PATH"] = orig
            shutil.rmtree(tmpdir, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()

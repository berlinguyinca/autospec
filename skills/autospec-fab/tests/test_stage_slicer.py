"""
test_stage_slicer.py — unittest for stage_slicer.py.

Tests:
  1. thick_wall.stl + printer.json  → status pass (wall >> min, no steep overhang)
  2. thin_wall.stl  + printer.json  → status fail + finding slicer_thin_wall
  3. steep_overhang.stl + printer.json → status fail + finding slicer_overhang
  4. fragment validates release-gate stage-record shape (stage/status/detail/findings)
  5. section detail present in fragment
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_SCRIPTS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "scripts"))
_FIXTURES_DIR = os.path.normpath(os.path.join(_THIS_DIR, "fixtures", "slicer"))

_STAGE_SCRIPT = os.path.join(_SCRIPTS_DIR, "stage_slicer.py")

_THICK_WALL_STL = os.path.join(_FIXTURES_DIR, "thick_wall.stl")
_THIN_WALL_STL = os.path.join(_FIXTURES_DIR, "thin_wall.stl")
_STEEP_OVERHANG_STL = os.path.join(_FIXTURES_DIR, "steep_overhang.stl")
_PRINTER_JSON = os.path.join(_FIXTURES_DIR, "printer.json")


def _run_stage(stl_path, extra_args=None):
    """Run stage_slicer.py on *stl_path*, return (returncode, fragment_dict, stderr)."""
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
        out_path = tf.name
    try:
        cmd = [
            sys.executable, _STAGE_SCRIPT,
            "--in", stl_path,
            "--out", out_path,
            "--printer", _PRINTER_JSON,
        ]
        if extra_args:
            cmd.extend(extra_args)
        result = subprocess.run(cmd, capture_output=True, text=True)
        fragment = None
        if os.path.exists(out_path):
            with open(out_path) as f:
                fragment = json.load(f)
        return result.returncode, fragment, result.stderr
    finally:
        if os.path.exists(out_path):
            os.unlink(out_path)


def _finding_codes(fragment):
    """Return set of code_health ids from fragment findings."""
    return {f.get("code") for f in fragment.get("findings", [])}


class TestFragmentShape(unittest.TestCase):
    """Every fragment must match release-gate stage-record shape."""

    def _assert_valid_fragment(self, fragment, stl_label):
        self.assertIsInstance(fragment, dict, f"{stl_label}: fragment must be a dict")
        for key in ("stage", "status", "detail", "findings"):
            self.assertIn(key, fragment, f"{stl_label}: fragment missing '{key}'")
        self.assertIn(
            fragment["status"], {"pass", "fail", "warn", "skip"},
            f"{stl_label}: invalid status {fragment['status']!r}",
        )
        self.assertIsInstance(fragment["findings"], list,
                              f"{stl_label}: 'findings' must be a list")
        self.assertEqual(fragment["stage"], "slicer",
                         f"{stl_label}: stage name must be 'slicer'")

    def test_thick_wall_fragment_shape(self):
        _, fragment, _ = _run_stage(_THICK_WALL_STL)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment, "thick_wall")

    def test_thin_wall_fragment_shape(self):
        _, fragment, _ = _run_stage(_THIN_WALL_STL)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment, "thin_wall")

    def test_steep_overhang_fragment_shape(self):
        _, fragment, _ = _run_stage(_STEEP_OVERHANG_STL)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment, "steep_overhang")


class TestThickWall(unittest.TestCase):
    """thick_wall.stl is a 20×20×20mm box — walls >> min, no steep overhang."""

    def setUp(self):
        self.rc, self.fragment, self.stderr = _run_stage(_THICK_WALL_STL)

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0,
                         f"Expected exit 0 (harness success); stderr={self.stderr}")

    def test_status_pass(self):
        self.assertEqual(
            self.fragment["status"], "pass",
            f"Expected pass for thick-wall box; fragment={self.fragment}",
        )

    def test_no_thin_wall_finding(self):
        codes = _finding_codes(self.fragment)
        self.assertNotIn("slicer_thin_wall", codes,
                         f"Unexpected slicer_thin_wall finding; findings={self.fragment['findings']}")

    def test_no_overhang_finding(self):
        codes = _finding_codes(self.fragment)
        self.assertNotIn("slicer_overhang", codes,
                         f"Unexpected slicer_overhang finding; findings={self.fragment['findings']}")

    def test_section_detail_present(self):
        self.assertIn("section", self.fragment["detail"].lower(),
                      f"Expected 'section' in detail; detail={self.fragment['detail']!r}")


class TestThinWall(unittest.TestCase):
    """thin_wall.stl is a 0.3mm-thick plate — below min wall (3 × 0.4 = 1.2mm)."""

    def setUp(self):
        self.rc, self.fragment, self.stderr = _run_stage(_THIN_WALL_STL)

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0,
                         f"Expected exit 0 (harness success); stderr={self.stderr}")

    def test_status_fail(self):
        self.assertEqual(
            self.fragment["status"], "fail",
            f"Expected fail for thin wall; fragment={self.fragment}",
        )

    def test_finding_slicer_thin_wall(self):
        codes = _finding_codes(self.fragment)
        self.assertIn("slicer_thin_wall", codes,
                      f"Expected slicer_thin_wall finding; findings={self.fragment['findings']}")


class TestSteepOverhang(unittest.TestCase):
    """steep_overhang.stl has a face whose normal points >45° from vertical downward."""

    def setUp(self):
        self.rc, self.fragment, self.stderr = _run_stage(_STEEP_OVERHANG_STL)

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0,
                         f"Expected exit 0 (harness success); stderr={self.stderr}")

    def test_status_fail(self):
        self.assertEqual(
            self.fragment["status"], "fail",
            f"Expected fail for steep overhang; fragment={self.fragment}",
        )

    def test_finding_slicer_overhang(self):
        codes = _finding_codes(self.fragment)
        self.assertIn("slicer_overhang", codes,
                      f"Expected slicer_overhang finding; findings={self.fragment['findings']}")


class TestMissingInArg(unittest.TestCase):
    """Missing --in should exit non-zero (harness/usage error)."""

    def test_missing_in_exits_nonzero(self):
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        try:
            result = subprocess.run(
                [sys.executable, _STAGE_SCRIPT, "--out", out_path],
                capture_output=True, text=True,
            )
            self.assertNotEqual(result.returncode, 0,
                                "Expected non-zero exit when --in is missing")
        finally:
            if os.path.exists(out_path):
                os.unlink(out_path)


class TestDefaultPrinterProfile(unittest.TestCase):
    """Without --printer, defaults (3×0.4=1.2mm min wall) still flag thin wall."""

    def test_thin_wall_fails_without_printer_arg(self):
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        try:
            cmd = [
                sys.executable, _STAGE_SCRIPT,
                "--in", _THIN_WALL_STL,
                "--out", out_path,
            ]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0,
                             f"Harness should exit 0; stderr={result.stderr}")
            with open(out_path) as f:
                fragment = json.load(f)
            self.assertEqual(fragment["status"], "fail",
                             f"Expected fail with default profile; fragment={fragment}")
            codes = _finding_codes(fragment)
            self.assertIn("slicer_thin_wall", codes,
                          f"Expected slicer_thin_wall; findings={fragment['findings']}")
        finally:
            if os.path.exists(out_path):
                os.unlink(out_path)


if __name__ == "__main__":
    unittest.main()

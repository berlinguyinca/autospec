"""
test_stage_geometry.py — unittest for stage-geometry.py.

Tests:
  1. box.stl   → status pass, watertight, 1 body
  2. open-cube.stl → status fail + finding non_watertight
  3. two-cubes.stl → status fail + finding disconnected_bodies (--expect-bodies 1)
  4. emitted fragment validates against release-gate stage-record shape
     (keys present, status is one of pass|fail|warn|skip)
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_SCRIPTS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "scripts"))
_FIXTURES_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "fixtures"))

_STAGE_SCRIPT = os.path.join(_SCRIPTS_DIR, "stage-geometry.py")

# Fixture paths
_BOX_STL = os.path.join(_FIXTURES_DIR, "box", "box.stl")
_OPEN_CUBE_STL = os.path.join(_FIXTURES_DIR, "open-cube.stl")
_TWO_CUBES_STL = os.path.join(_FIXTURES_DIR, "two-cubes.stl")


def _run_stage(stl_path, extra_args=None):
    """Run stage-geometry.py on *stl_path*, return (returncode, fragment_dict)."""
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
        out_path = tf.name
    try:
        cmd = [sys.executable, _STAGE_SCRIPT, "--in", stl_path, "--out", out_path]
        if extra_args:
            cmd.extend(extra_args)
        result = subprocess.run(cmd, capture_output=True, text=True)
        if os.path.exists(out_path):
            with open(out_path) as f:
                fragment = json.load(f)
        else:
            fragment = None
        return result.returncode, fragment, result.stderr
    finally:
        if os.path.exists(out_path):
            os.unlink(out_path)


def _finding_codes(fragment):
    """Return set of code_health ids from fragment findings."""
    return {f.get("code") for f in fragment.get("findings", [])}


class TestFragmentShape(unittest.TestCase):
    """Stage fragment validates against release-gate stage-record shape."""

    def _assert_valid_fragment(self, fragment):
        self.assertIsInstance(fragment, dict, "fragment must be a dict")
        self.assertIn("stage", fragment, "fragment missing 'stage' key")
        self.assertIn("status", fragment, "fragment missing 'status' key")
        self.assertIn(fragment["status"], {"pass", "fail", "warn", "skip"},
                      f"invalid status: {fragment['status']!r}")
        self.assertIn("detail", fragment, "fragment missing 'detail' key")
        self.assertIn("findings", fragment, "fragment missing 'findings' key")
        self.assertIsInstance(fragment["findings"], list, "'findings' must be a list")
        self.assertEqual(fragment["stage"], "geometry", "stage name must be 'geometry'")

    def test_box_fragment_shape(self):
        _, fragment, _ = _run_stage(_BOX_STL)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment)

    def test_open_cube_fragment_shape(self):
        _, fragment, _ = _run_stage(_OPEN_CUBE_STL)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment)

    def test_two_cubes_fragment_shape(self):
        _, fragment, _ = _run_stage(_TWO_CUBES_STL)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment)


class TestBoxSTL(unittest.TestCase):
    """box.stl is a watertight 12-facet cube — should pass all checks."""

    def setUp(self):
        self.rc, self.fragment, self.stderr = _run_stage(_BOX_STL)

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0,
                         f"Expected exit 0 (harness success); stderr={self.stderr}")

    def test_status_pass(self):
        self.assertEqual(self.fragment["status"], "pass",
                         f"Expected pass; fragment={self.fragment}")

    def test_no_findings(self):
        codes = _finding_codes(self.fragment)
        self.assertNotIn("non_watertight", codes)
        self.assertNotIn("disconnected_bodies", codes)

    def test_fixture_exists(self):
        self.assertTrue(os.path.exists(_BOX_STL), f"Missing: {_BOX_STL}")


class TestOpenCubeSTL(unittest.TestCase):
    """open-cube.stl has one facet removed — NOT watertight."""

    def setUp(self):
        self.rc, self.fragment, self.stderr = _run_stage(_OPEN_CUBE_STL)

    def test_exit_code_zero(self):
        # Harness exit 0 even on geometry fail (verdict is in status)
        self.assertEqual(self.rc, 0,
                         f"Expected exit 0 (harness success); stderr={self.stderr}")

    def test_status_fail(self):
        self.assertEqual(self.fragment["status"], "fail",
                         f"Expected fail for non-watertight; fragment={self.fragment}")

    def test_finding_non_watertight(self):
        codes = _finding_codes(self.fragment)
        self.assertIn("non_watertight", codes,
                      f"Expected non_watertight finding; findings={self.fragment['findings']}")


class TestTwoCubesSTL(unittest.TestCase):
    """two-cubes.stl has two disjoint bodies — disconnected with --expect-bodies 1."""

    def setUp(self):
        self.rc, self.fragment, self.stderr = _run_stage(
            _TWO_CUBES_STL, extra_args=["--expect-bodies", "1"]
        )

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0,
                         f"Expected exit 0 (harness success); stderr={self.stderr}")

    def test_status_fail(self):
        self.assertEqual(self.fragment["status"], "fail",
                         f"Expected fail for disconnected bodies; fragment={self.fragment}")

    def test_finding_disconnected_bodies(self):
        codes = _finding_codes(self.fragment)
        self.assertIn("disconnected_bodies", codes,
                      f"Expected disconnected_bodies finding; findings={self.fragment['findings']}")


class TestTwoCubesExpectTwo(unittest.TestCase):
    """two-cubes.stl passes when --expect-bodies 2."""

    def setUp(self):
        self.rc, self.fragment, self.stderr = _run_stage(
            _TWO_CUBES_STL, extra_args=["--expect-bodies", "2"]
        )

    def test_status_pass(self):
        self.assertEqual(self.fragment["status"], "pass",
                         f"Expected pass with --expect-bodies 2; fragment={self.fragment}")


class TestMissingInArg(unittest.TestCase):
    """Missing --in should exit non-zero (harness/usage error)."""

    def test_missing_in_exits_nonzero(self):
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        try:
            result = subprocess.run(
                [sys.executable, _STAGE_SCRIPT, "--out", out_path],
                capture_output=True, text=True
            )
            self.assertNotEqual(result.returncode, 0,
                                "Expected non-zero exit when --in is missing")
        finally:
            if os.path.exists(out_path):
                os.unlink(out_path)


if __name__ == "__main__":
    unittest.main()

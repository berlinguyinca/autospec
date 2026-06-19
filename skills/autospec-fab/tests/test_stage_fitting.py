"""
test_stage_fitting.py — unittest for stage_fitting.py.

Tests:
  1. port-good fixture   → status pass (clearance≥5, tap_access, integrated body)
  2. port-blocked fixture → status fail + finding blocked_npt (clearance<5 AND tap_access false)
  3. clearance-only fail  → status fail + blocked_npt (clearance_mm=3, tap_access true)
  4. port-tower fixture   → status fail + finding blocked_npt (standalone_tower)
  5. fragment shape       → stage="vacuum-fitting", status in {pass,fail,warn,skip},
                            keys: stage, status, detail, findings
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_SCRIPTS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "scripts"))
_FIXTURES_DIR = os.path.normpath(os.path.join(_THIS_DIR, "fixtures"))

_STAGE_SCRIPT = os.path.join(_SCRIPTS_DIR, "stage_fitting.py")

_GOOD_STL   = os.path.join(_FIXTURES_DIR, "port-good",    "body.stl")
_GOOD_MODEL = os.path.join(_FIXTURES_DIR, "port-good",    "model.json")

_BLOCKED_STL   = os.path.join(_FIXTURES_DIR, "port-blocked", "body.stl")
_BLOCKED_MODEL = os.path.join(_FIXTURES_DIR, "port-blocked", "model.json")

_TOWER_STL   = os.path.join(_FIXTURES_DIR, "port-tower",  "body.stl")
_TOWER_MODEL = os.path.join(_FIXTURES_DIR, "port-tower",  "model.json")


def _run_stage(stl_path, model_path, extra_args=None):
    """Run stage_fitting.py and return (returncode, fragment_dict, stderr)."""
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
        out_path = tf.name
    try:
        cmd = [
            sys.executable, _STAGE_SCRIPT,
            "--in", stl_path,
            "--model", model_path,
            "--out", out_path,
        ]
        if extra_args:
            cmd.extend(extra_args)
        result = subprocess.run(cmd, capture_output=True, text=True)
        fragment = None
        if os.path.exists(out_path):
            with open(out_path) as fh:
                fragment = json.load(fh)
        return result.returncode, fragment, result.stderr
    finally:
        if os.path.exists(out_path):
            os.unlink(out_path)


def _finding_codes(fragment):
    """Return set of code_health ids from fragment findings list."""
    return {f.get("code") for f in fragment.get("findings", [])}


class TestFragmentShape(unittest.TestCase):
    """Every invocation emits a valid release-gate stage-record fragment."""

    def _assert_valid_fragment(self, fragment):
        self.assertIsInstance(fragment, dict)
        for key in ("stage", "status", "detail", "findings"):
            self.assertIn(key, fragment, f"fragment missing key '{key}'")
        self.assertIn(
            fragment["status"], {"pass", "fail", "warn", "skip"},
            f"invalid status: {fragment['status']!r}",
        )
        self.assertEqual(fragment["stage"], "vacuum-fitting")
        self.assertIsInstance(fragment["findings"], list)

    def test_good_fragment_shape(self):
        _, frag, _ = _run_stage(_GOOD_STL, _GOOD_MODEL)
        self.assertIsNotNone(frag, "no fragment emitted for good port")
        self._assert_valid_fragment(frag)

    def test_blocked_fragment_shape(self):
        _, frag, _ = _run_stage(_BLOCKED_STL, _BLOCKED_MODEL)
        self.assertIsNotNone(frag, "no fragment emitted for blocked port")
        self._assert_valid_fragment(frag)

    def test_tower_fragment_shape(self):
        _, frag, _ = _run_stage(_TOWER_STL, _TOWER_MODEL)
        self.assertIsNotNone(frag, "no fragment emitted for tower port")
        self._assert_valid_fragment(frag)


class TestCompliantPort(unittest.TestCase):
    """A well-formed NPT port in a solid integrated body → pass."""

    def test_good_port_passes(self):
        rc, frag, stderr = _run_stage(_GOOD_STL, _GOOD_MODEL)
        self.assertEqual(rc, 0, f"harness error: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(frag["status"], "pass",
                         f"expected pass, got {frag['status']!r}; detail={frag.get('detail')}")

    def test_good_port_no_blocked_npt(self):
        _, frag, _ = _run_stage(_GOOD_STL, _GOOD_MODEL)
        self.assertIsNotNone(frag)
        codes = _finding_codes(frag)
        self.assertNotIn("blocked_npt", codes,
                         "compliant port must not produce blocked_npt")


class TestBlockedAccess(unittest.TestCase):
    """Port with tap_access false and clearance_mm<5 → fail + blocked_npt."""

    def test_blocked_port_fails(self):
        rc, frag, stderr = _run_stage(_BLOCKED_STL, _BLOCKED_MODEL)
        self.assertEqual(rc, 0, f"harness error: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(frag["status"], "fail",
                         f"expected fail, got {frag['status']!r}")

    def test_blocked_port_emits_blocked_npt(self):
        _, frag, _ = _run_stage(_BLOCKED_STL, _BLOCKED_MODEL)
        self.assertIsNotNone(frag)
        self.assertIn("blocked_npt", _finding_codes(frag),
                      "blocked port must emit blocked_npt finding")


class TestClearanceOnly(unittest.TestCase):
    """Port with clearance_mm<5 but tap_access true → still fail blocked_npt."""

    def test_low_clearance_fails(self):
        # Build a temp model with clearance_mm=3 but tap_access=true
        model = {
            "material": "PETG",
            "print_orientation": "flat",
            "load_critical": False,
            "flow_critical": True,
            "ports": [
                {
                    "id": "p1",
                    "type": "npt",
                    "size": "1/4",
                    "clearance_mm": 3.0,
                    "tap_access": True,
                    "thread_depth_mm": 6.0,
                }
            ],
        }
        with tempfile.NamedTemporaryFile(
            suffix=".json", mode="w", delete=False
        ) as tf:
            json.dump(model, tf)
            tmp_model = tf.name
        try:
            rc, frag, stderr = _run_stage(_GOOD_STL, tmp_model)
            self.assertEqual(rc, 0, f"harness error: {stderr}")
            self.assertIsNotNone(frag)
            self.assertEqual(frag["status"], "fail",
                             "clearance_mm=3 with tap_access=true must still fail")
            self.assertIn("blocked_npt", _finding_codes(frag))
        finally:
            os.unlink(tmp_model)


class TestNarrowTower(unittest.TestCase):
    """NPT port on a narrow standalone tower → fail + blocked_npt."""

    def test_tower_port_fails(self):
        rc, frag, stderr = _run_stage(_TOWER_STL, _TOWER_MODEL)
        self.assertEqual(rc, 0, f"harness error: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(frag["status"], "fail",
                         f"expected fail for tower port, got {frag['status']!r}")

    def test_tower_port_emits_blocked_npt(self):
        _, frag, _ = _run_stage(_TOWER_STL, _TOWER_MODEL)
        self.assertIsNotNone(frag)
        self.assertIn("blocked_npt", _finding_codes(frag),
                      "narrow-tower port must emit blocked_npt")

    def test_tower_detected_by_geometry(self):
        """Geometry-only detection: same STL + model without standalone_tower flag
        must still be flagged because the STL aspect ratio exceeds threshold."""
        model = {
            "material": "PETG",
            "print_orientation": "upright",
            "load_critical": False,
            "flow_critical": True,
            "dims": {"x_mm": 4, "y_mm": 4, "z_mm": 40},
            "ports": [
                {
                    "id": "p1",
                    "type": "npt",
                    "size": "1/4",
                    "clearance_mm": 8.0,
                    "tap_access": True,
                    "thread_depth_mm": 6.0,
                    # no standalone_tower key
                }
            ],
        }
        with tempfile.NamedTemporaryFile(
            suffix=".json", mode="w", delete=False
        ) as tf:
            json.dump(model, tf)
            tmp_model = tf.name
        try:
            rc, frag, stderr = _run_stage(_TOWER_STL, tmp_model)
            self.assertEqual(rc, 0, f"harness error: {stderr}")
            self.assertIsNotNone(frag)
            self.assertEqual(frag["status"], "fail",
                             "narrow-tower geometry alone must trigger fail")
            self.assertIn("blocked_npt", _finding_codes(frag))
        finally:
            os.unlink(tmp_model)


if __name__ == "__main__":
    unittest.main()

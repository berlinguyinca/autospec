"""
test_stage_gasket.py — unittest for stage_gasket.py.

Tests:
  1. gasket-enclosed fixture → status pass (surrounding_wall≥5, seal continuous,
                               not exposed)
  2. gasket-thin fixture     → status fail + finding exposed_gasket (wall < 5 mm)
  3. gasket-exposed fixture  → status fail + finding exposed_gasket (exposed_side
                               true + broken seal face)
  4. fragment shape          → stage="gasket", status in {pass,fail,warn,skip},
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

_STAGE_SCRIPT = os.path.join(_SCRIPTS_DIR, "stage_gasket.py")

_ENCLOSED_STL   = os.path.join(_FIXTURES_DIR, "gasket-enclosed", "body.stl")
_ENCLOSED_MODEL = os.path.join(_FIXTURES_DIR, "gasket-enclosed", "model.json")

_THIN_STL   = os.path.join(_FIXTURES_DIR, "gasket-thin", "body.stl")
_THIN_MODEL = os.path.join(_FIXTURES_DIR, "gasket-thin", "model.json")

_EXPOSED_STL   = os.path.join(_FIXTURES_DIR, "gasket-exposed", "body.stl")
_EXPOSED_MODEL = os.path.join(_FIXTURES_DIR, "gasket-exposed", "model.json")


def _run_stage(stl_path, model_path, extra_args=None):
    """Run stage_gasket.py and return (returncode, fragment_dict, stderr)."""
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
        self.assertEqual(fragment["stage"], "gasket")
        self.assertIsInstance(fragment["findings"], list)

    def test_enclosed_fragment_shape(self):
        _, frag, _ = _run_stage(_ENCLOSED_STL, _ENCLOSED_MODEL)
        self.assertIsNotNone(frag, "no fragment emitted for enclosed gasket")
        self._assert_valid_fragment(frag)

    def test_thin_fragment_shape(self):
        _, frag, _ = _run_stage(_THIN_STL, _THIN_MODEL)
        self.assertIsNotNone(frag, "no fragment emitted for thin-wall gasket")
        self._assert_valid_fragment(frag)

    def test_exposed_fragment_shape(self):
        _, frag, _ = _run_stage(_EXPOSED_STL, _EXPOSED_MODEL)
        self.assertIsNotNone(frag, "no fragment emitted for exposed-side gasket")
        self._assert_valid_fragment(frag)


class TestEnclosedGasket(unittest.TestCase):
    """Well-enclosed gasket groove with ≥5 mm wall, continuous seal → pass."""

    def test_enclosed_passes(self):
        rc, frag, stderr = _run_stage(_ENCLOSED_STL, _ENCLOSED_MODEL)
        self.assertEqual(rc, 0, f"harness error: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(
            frag["status"], "pass",
            f"expected pass, got {frag['status']!r}; detail={frag.get('detail')}",
        )

    def test_enclosed_no_exposed_gasket(self):
        _, frag, _ = _run_stage(_ENCLOSED_STL, _ENCLOSED_MODEL)
        self.assertIsNotNone(frag)
        codes = _finding_codes(frag)
        self.assertNotIn(
            "exposed_gasket", codes,
            "compliant enclosed gasket must not produce exposed_gasket finding",
        )


class TestThinWallGasket(unittest.TestCase):
    """Gasket with surrounding_wall_mm < 5 → fail + exposed_gasket."""

    def test_thin_wall_fails(self):
        rc, frag, stderr = _run_stage(_THIN_STL, _THIN_MODEL)
        self.assertEqual(rc, 0, f"harness error: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(
            frag["status"], "fail",
            f"expected fail for thin-wall gasket, got {frag['status']!r}",
        )

    def test_thin_wall_emits_exposed_gasket(self):
        _, frag, _ = _run_stage(_THIN_STL, _THIN_MODEL)
        self.assertIsNotNone(frag)
        self.assertIn(
            "exposed_gasket", _finding_codes(frag),
            "thin-wall gasket must emit exposed_gasket finding",
        )


class TestExposedSideGasket(unittest.TestCase):
    """Gasket with exposed_side true or broken seal face → fail + exposed_gasket."""

    def test_exposed_side_fails(self):
        rc, frag, stderr = _run_stage(_EXPOSED_STL, _EXPOSED_MODEL)
        self.assertEqual(rc, 0, f"harness error: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(
            frag["status"], "fail",
            f"expected fail for exposed-side gasket, got {frag['status']!r}",
        )

    def test_exposed_side_emits_exposed_gasket(self):
        _, frag, _ = _run_stage(_EXPOSED_STL, _EXPOSED_MODEL)
        self.assertIsNotNone(frag)
        self.assertIn(
            "exposed_gasket", _finding_codes(frag),
            "exposed-side gasket must emit exposed_gasket finding",
        )


class TestInlineModels(unittest.TestCase):
    """Boundary/inline model checks without fixture files."""

    def test_wall_exactly_5mm_passes(self):
        """surrounding_wall_mm == 5.0 is on the boundary → should pass."""
        model = {
            "material": "PETG",
            "print_orientation": "flat",
            "load_critical": False,
            "flow_critical": True,
            "gaskets": [
                {
                    "id": "g1",
                    "groove": "o-ring",
                    "surrounding_wall_mm": 5.0,
                    "seal_face_continuous": True,
                    "exposed_side": False,
                }
            ],
        }
        with tempfile.NamedTemporaryFile(
            suffix=".json", mode="w", delete=False
        ) as tf:
            json.dump(model, tf)
            tmp_model = tf.name
        try:
            rc, frag, stderr = _run_stage(_ENCLOSED_STL, tmp_model)
            self.assertEqual(rc, 0, f"harness error: {stderr}")
            self.assertIsNotNone(frag)
            self.assertEqual(
                frag["status"], "pass",
                "surrounding_wall_mm==5.0 must pass (boundary inclusive)",
            )
        finally:
            os.unlink(tmp_model)

    def test_broken_seal_face_alone_fails(self):
        """seal_face_continuous false with adequate wall → fail exposed_gasket."""
        model = {
            "material": "PETG",
            "print_orientation": "flat",
            "load_critical": False,
            "flow_critical": True,
            "gaskets": [
                {
                    "id": "g1",
                    "groove": "o-ring",
                    "surrounding_wall_mm": 8.0,
                    "seal_face_continuous": False,
                    "exposed_side": False,
                }
            ],
        }
        with tempfile.NamedTemporaryFile(
            suffix=".json", mode="w", delete=False
        ) as tf:
            json.dump(model, tf)
            tmp_model = tf.name
        try:
            rc, frag, stderr = _run_stage(_ENCLOSED_STL, tmp_model)
            self.assertEqual(rc, 0, f"harness error: {stderr}")
            self.assertIsNotNone(frag)
            self.assertEqual(frag["status"], "fail",
                             "broken seal face must fail even with good wall")
            self.assertIn("exposed_gasket", _finding_codes(frag))
        finally:
            os.unlink(tmp_model)

    def test_no_gaskets_passes(self):
        """Model with empty gaskets list → pass (nothing to reject)."""
        model = {
            "material": "PETG",
            "print_orientation": "flat",
            "load_critical": False,
            "flow_critical": False,
            "gaskets": [],
        }
        with tempfile.NamedTemporaryFile(
            suffix=".json", mode="w", delete=False
        ) as tf:
            json.dump(model, tf)
            tmp_model = tf.name
        try:
            rc, frag, stderr = _run_stage(_ENCLOSED_STL, tmp_model)
            self.assertEqual(rc, 0, f"harness error: {stderr}")
            self.assertIsNotNone(frag)
            self.assertEqual(frag["status"], "pass",
                             "empty gaskets list must pass")
        finally:
            os.unlink(tmp_model)


if __name__ == "__main__":
    unittest.main()

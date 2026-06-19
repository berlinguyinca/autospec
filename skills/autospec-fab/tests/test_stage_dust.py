"""
test_stage_dust.py — unittest for stage_dust.py.

Tests:
  1. clean fixture        → status pass (full-size openings, monotonic area,
                             connected, no gate-slot, no PVC)
  2. pinched fixture      → status fail (cross-section drops below 0.6× nominal
                             area mid-path)
  3. blocked fixture      → status fail (opening_mm < min_opening_mm AND
                             connected false)
  4. gate-slot fixture    → status fail (printed_gate_slot true)
  5. pvc fixture          → status fail (pvc_as_duct true)
  6. fragment shape       → stage="dust-airflow", status in {pass,fail,warn,skip},
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
_FIXTURES_DIR = os.path.normpath(os.path.join(_THIS_DIR, "fixtures", "dust"))

_STAGE_SCRIPT = os.path.join(_SCRIPTS_DIR, "stage_dust.py")

# Reuse a small existing STL (dust stage doesn't gate on geometry unless STL probe)
_SHARED_STL = os.path.normpath(
    os.path.join(_THIS_DIR, "fixtures", "port-good", "body.stl")
)

_CLEAN_DUCT     = os.path.join(_FIXTURES_DIR, "clean.json")
_PINCHED_DUCT   = os.path.join(_FIXTURES_DIR, "pinched.json")
_BLOCKED_DUCT   = os.path.join(_FIXTURES_DIR, "blocked.json")
_GATE_SLOT_DUCT = os.path.join(_FIXTURES_DIR, "gate-slot.json")
_PVC_DUCT       = os.path.join(_FIXTURES_DIR, "pvc.json")


def _run_stage(duct_path, extra_args=None):
    """Run stage_dust.py and return (returncode, fragment_dict, stderr)."""
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
        out_path = tf.name
    try:
        cmd = [
            sys.executable, _STAGE_SCRIPT,
            "--in", _SHARED_STL,
            "--duct", duct_path,
            "--out", out_path,
        ]
        if extra_args:
            cmd.extend(extra_args)
        result = subprocess.run(cmd, capture_output=True, text=True)
        fragment = None
        if os.path.exists(out_path):
            with open(out_path) as fh:
                try:
                    fragment = json.load(fh)
                except json.JSONDecodeError:
                    pass
        return result.returncode, fragment, result.stderr
    finally:
        if os.path.exists(out_path):
            os.unlink(out_path)


def _finding_codes(fragment):
    """Return set of code values from fragment findings list."""
    return {f.get("code") for f in fragment.get("findings", [])}


# ---------------------------------------------------------------------------
# Fragment shape contract
# ---------------------------------------------------------------------------

class TestFragmentShape(unittest.TestCase):
    """Every invocation emits a valid release-gate stage-record fragment."""

    VALID_STATUSES = {"pass", "fail", "warn", "skip"}
    REQUIRED_KEYS = {"stage", "status", "detail", "findings"}

    def test_clean_fragment_shape(self):
        rc, fragment, stderr = _run_stage(_CLEAN_DUCT)
        self.assertEqual(rc, 0, f"stage exited non-zero: {stderr}")
        self.assertIsNotNone(fragment, "fragment JSON not written")
        self.assertEqual(fragment.get("stage"), "dust-airflow")
        self.assertIn(fragment.get("status"), self.VALID_STATUSES)
        self.assertGreaterEqual(
            set(fragment.keys()), self.REQUIRED_KEYS,
            f"missing keys: {self.REQUIRED_KEYS - set(fragment.keys())}",
        )
        self.assertIsInstance(fragment.get("findings"), list)


# ---------------------------------------------------------------------------
# Passing case
# ---------------------------------------------------------------------------

class TestCleanDuct(unittest.TestCase):
    """Full-size, connected, monotonic, no gate-slot, no PVC → pass."""

    def test_status_pass(self):
        rc, fragment, stderr = _run_stage(_CLEAN_DUCT)
        self.assertEqual(rc, 0, f"stage exited non-zero: {stderr}")
        self.assertIsNotNone(fragment)
        self.assertEqual(fragment.get("status"), "pass",
                         f"expected pass, got: {fragment}")

    def test_no_findings(self):
        _rc, fragment, _stderr = _run_stage(_CLEAN_DUCT)
        self.assertIsNotNone(fragment)
        self.assertEqual(fragment.get("findings"), [],
                         f"expected empty findings, got: {fragment.get('findings')}")


# ---------------------------------------------------------------------------
# Pinched cross-section
# ---------------------------------------------------------------------------

class TestPinchedDuct(unittest.TestCase):
    """Cross-section drops to <60% of nominal inlet area mid-path → fail."""

    def test_status_fail(self):
        _rc, fragment, _stderr = _run_stage(_PINCHED_DUCT)
        self.assertIsNotNone(fragment)
        self.assertEqual(fragment.get("status"), "fail",
                         f"expected fail for pinched duct: {fragment}")

    def test_finding_code(self):
        _rc, fragment, _stderr = _run_stage(_PINCHED_DUCT)
        self.assertIsNotNone(fragment)
        codes = _finding_codes(fragment)
        self.assertTrue(
            codes & {"disconnected_flow", "dust_obstructed"},
            f"expected disconnected_flow or dust_obstructed in findings, got: {codes}",
        )


# ---------------------------------------------------------------------------
# Blocked port / disconnected duct
# ---------------------------------------------------------------------------

class TestBlockedDuct(unittest.TestCase):
    """opening_mm < min_opening_mm AND connected=false → fail."""

    def test_status_fail(self):
        _rc, fragment, _stderr = _run_stage(_BLOCKED_DUCT)
        self.assertIsNotNone(fragment)
        self.assertEqual(fragment.get("status"), "fail",
                         f"expected fail for blocked duct: {fragment}")

    def test_finding_code_disconnected(self):
        _rc, fragment, _stderr = _run_stage(_BLOCKED_DUCT)
        self.assertIsNotNone(fragment)
        codes = _finding_codes(fragment)
        self.assertIn("disconnected_flow", codes,
                      f"expected disconnected_flow in findings, got: {codes}")


# ---------------------------------------------------------------------------
# Printed gate slot
# ---------------------------------------------------------------------------

class TestGateSlotDuct(unittest.TestCase):
    """printed_gate_slot=true → hard reject."""

    def test_status_fail(self):
        _rc, fragment, _stderr = _run_stage(_GATE_SLOT_DUCT)
        self.assertIsNotNone(fragment)
        self.assertEqual(fragment.get("status"), "fail",
                         f"expected fail for gate-slot duct: {fragment}")

    def test_finding_present(self):
        _rc, fragment, _stderr = _run_stage(_GATE_SLOT_DUCT)
        self.assertIsNotNone(fragment)
        self.assertTrue(
            len(fragment.get("findings", [])) > 0,
            "expected at least one finding for printed gate slot",
        )


# ---------------------------------------------------------------------------
# PVC as printed duct
# ---------------------------------------------------------------------------

class TestPvcDuct(unittest.TestCase):
    """pvc_as_duct=true → hard reject."""

    def test_status_fail(self):
        _rc, fragment, _stderr = _run_stage(_PVC_DUCT)
        self.assertIsNotNone(fragment)
        self.assertEqual(fragment.get("status"), "fail",
                         f"expected fail for PVC duct: {fragment}")

    def test_finding_present(self):
        _rc, fragment, _stderr = _run_stage(_PVC_DUCT)
        self.assertIsNotNone(fragment)
        self.assertTrue(
            len(fragment.get("findings", [])) > 0,
            "expected at least one finding for PVC-as-duct",
        )


class TestAbsentDuctSkips(unittest.TestCase):
    """Engine sequences stages with only --in/--model/--out (no --duct).

    A model with no dust-collection ducts must DEGRADE the stage to a
    non-blocking skip, not hard-fail — otherwise the release-gate engine can
    never reach a green gate for a duct-less model.
    """

    def _run_uniform(self, with_model=True):
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        try:
            cmd = [sys.executable, _STAGE_SCRIPT, "--in", _SHARED_STL,
                   "--out", out_path]
            if with_model:
                cmd.extend(["--model", _SHARED_STL])  # any path; unused
            result = subprocess.run(cmd, capture_output=True, text=True)
            frag = None
            if os.path.exists(out_path):
                with open(out_path) as fh:
                    frag = json.load(fh)
            return result.returncode, frag, result.stderr
        finally:
            if os.path.exists(out_path):
                os.unlink(out_path)

    def test_no_duct_skips(self):
        rc, frag, stderr = self._run_uniform(with_model=False)
        self.assertEqual(rc, 0, f"harness error without --duct: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(frag.get("status"), "skip")
        self.assertEqual(frag.get("stage"), "dust-airflow")

    def test_uniform_cli_with_model_skips(self):
        # Mirrors the engine's exact call: --in --model --out, no --duct.
        rc, frag, stderr = self._run_uniform(with_model=True)
        self.assertEqual(rc, 0, f"harness error on uniform CLI: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(frag.get("status"), "skip")


if __name__ == "__main__":
    unittest.main()

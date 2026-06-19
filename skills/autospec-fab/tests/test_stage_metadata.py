"""
test_stage_metadata.py — unittest for stage_metadata.py.

Tests:
  1. valid sidecar (model-valid.json)   → status pass
  2. invalid sidecar (model-invalid.json, missing material) → status fail + metadata_invalid
  3. absent sidecar (nonexistent path)  → status fail + metadata_missing (no ajv needed)
  4. fragment shape: required keys + valid status enum
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_SCRIPTS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "scripts"))
_FIXTURES_DIR = os.path.normpath(os.path.join(_THIS_DIR, "fixtures"))

_STAGE_SCRIPT = os.path.join(_SCRIPTS_DIR, "stage_metadata.py")

_VALID_SIDECAR = os.path.join(_FIXTURES_DIR, "model-valid.json")
_INVALID_SIDECAR = os.path.join(_FIXTURES_DIR, "model-invalid.json")
_ABSENT_SIDECAR = os.path.join(_FIXTURES_DIR, "nonexistent-model.json")

_AJV_AVAILABLE = shutil.which("ajv") is not None


def _run_stage(model_path):
    """Run stage_metadata.py --model <path> --out <tmp>, return (rc, fragment, stderr)."""
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
        out_path = tf.name
    try:
        cmd = [sys.executable, _STAGE_SCRIPT, "--model", model_path, "--out", out_path]
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
    return {f.get("code") for f in fragment.get("findings", [])}


# ---------------------------------------------------------------------------
# Fragment shape
# ---------------------------------------------------------------------------

class TestFragmentShape(unittest.TestCase):
    """Every emitted fragment has the required stage-record keys."""

    _VALID_STATUSES = {"pass", "fail", "warn", "skip"}

    def _assert_valid_fragment(self, fragment, expected_stage="metadata"):
        self.assertIsInstance(fragment, dict, "fragment must be a dict")
        self.assertIn("stage", fragment)
        self.assertIn("status", fragment)
        self.assertIn(fragment["status"], self._VALID_STATUSES,
                      f"invalid status: {fragment['status']!r}")
        self.assertIn("detail", fragment)
        self.assertIn("findings", fragment)
        self.assertIsInstance(fragment["findings"], list)
        self.assertEqual(fragment["stage"], expected_stage)

    def test_valid_sidecar_fragment_shape(self):
        if not _AJV_AVAILABLE:
            self.skipTest("ajv not on PATH — skip ajv-dependent shape check")
        _, fragment, _ = _run_stage(_VALID_SIDECAR)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment)

    def test_invalid_sidecar_fragment_shape(self):
        if not _AJV_AVAILABLE:
            self.skipTest("ajv not on PATH — skip ajv-dependent shape check")
        _, fragment, _ = _run_stage(_INVALID_SIDECAR)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment)

    def test_absent_sidecar_fragment_shape(self):
        # No ajv needed — sidecar-missing check runs before ajv
        _, fragment, _ = _run_stage(_ABSENT_SIDECAR)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment)


# ---------------------------------------------------------------------------
# Valid sidecar → pass
# ---------------------------------------------------------------------------

class TestValidSidecar(unittest.TestCase):
    """model-valid.json has all required fields → should pass."""

    def setUp(self):
        if not _AJV_AVAILABLE:
            self.skipTest("ajv not on PATH — skip ajv-dependent test")
        self.rc, self.fragment, self.stderr = _run_stage(_VALID_SIDECAR)

    def test_fixture_exists(self):
        self.assertTrue(os.path.exists(_VALID_SIDECAR), f"Missing: {_VALID_SIDECAR}")

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0, f"Expected exit 0; stderr={self.stderr}")

    def test_status_pass(self):
        self.assertEqual(self.fragment["status"], "pass",
                         f"Expected pass; fragment={self.fragment}")

    def test_no_findings(self):
        codes = _finding_codes(self.fragment)
        self.assertNotIn("metadata_invalid", codes)
        self.assertNotIn("metadata_missing", codes)


# ---------------------------------------------------------------------------
# Invalid sidecar → fail + metadata_invalid
# ---------------------------------------------------------------------------

class TestInvalidSidecar(unittest.TestCase):
    """model-invalid.json is missing 'material' → should fail with metadata_invalid."""

    def setUp(self):
        if not _AJV_AVAILABLE:
            self.skipTest("ajv not on PATH — skip ajv-dependent test")
        self.rc, self.fragment, self.stderr = _run_stage(_INVALID_SIDECAR)

    def test_fixture_exists(self):
        self.assertTrue(os.path.exists(_INVALID_SIDECAR), f"Missing: {_INVALID_SIDECAR}")

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0, f"Expected exit 0; stderr={self.stderr}")

    def test_status_fail(self):
        self.assertEqual(self.fragment["status"], "fail",
                         f"Expected fail; fragment={self.fragment}")

    def test_finding_metadata_invalid(self):
        codes = _finding_codes(self.fragment)
        self.assertIn("metadata_invalid", codes,
                      f"Expected metadata_invalid finding; findings={self.fragment['findings']}")


# ---------------------------------------------------------------------------
# Absent sidecar → fail + metadata_missing  (no ajv required)
# ---------------------------------------------------------------------------

class TestAbsentSidecar(unittest.TestCase):
    """Pointing --model at a nonexistent path → fail + metadata_missing."""

    def setUp(self):
        # This test does NOT need ajv — the missing-file check runs before ajv.
        self.rc, self.fragment, self.stderr = _run_stage(_ABSENT_SIDECAR)

    def test_sidecar_truly_absent(self):
        self.assertFalse(os.path.exists(_ABSENT_SIDECAR),
                         f"Fixture should not exist: {_ABSENT_SIDECAR}")

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0, f"Expected exit 0; stderr={self.stderr}")

    def test_status_fail(self):
        self.assertEqual(self.fragment["status"], "fail",
                         f"Expected fail for absent sidecar; fragment={self.fragment}")

    def test_finding_metadata_missing(self):
        codes = _finding_codes(self.fragment)
        self.assertIn("metadata_missing", codes,
                      f"Expected metadata_missing finding; findings={self.fragment['findings']}")


# ---------------------------------------------------------------------------
# Missing --model arg → non-zero exit (harness/usage error)
# ---------------------------------------------------------------------------

class TestMissingModelArg(unittest.TestCase):
    """Omitting --model should exit non-zero (usage error)."""

    def test_missing_model_exits_nonzero(self):
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        try:
            result = subprocess.run(
                [sys.executable, _STAGE_SCRIPT, "--out", out_path],
                capture_output=True, text=True,
            )
            self.assertNotEqual(result.returncode, 0,
                                "Expected non-zero exit when --model is missing")
        finally:
            if os.path.exists(out_path):
                os.unlink(out_path)


if __name__ == "__main__":
    unittest.main()

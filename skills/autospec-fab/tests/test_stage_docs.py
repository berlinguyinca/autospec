"""
test_stage_docs.py — unittest for stage_docs.py.

Real file diffs, no mocks. Drives the stage via its uniform CLI against three
committed fixtures under fixtures/docs/:

  1. in-sync/        → status pass  (all generated artifacts match baseline.json;
                       all hand docs reference every current fact).
  2. stale-manifest/ → status fail + finding docs_stale
                       (MANIFEST.md omits the current model 'dust_deputy';
                        generated artifacts still match baseline).
  3. hand-edited/    → status fail + finding hand_edited_generated
                       (a build/ STL was hand-edited so its sha256 no longer
                        matches baseline.json; hand docs are still in sync).
  4. fragment shape  → required stage-record keys + valid status enum.
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
_FIXTURES_DIR = os.path.normpath(os.path.join(_THIS_DIR, "fixtures", "docs"))

_STAGE_SCRIPT = os.path.join(_SCRIPTS_DIR, "stage_docs.py")

_IN_SYNC = os.path.join(_FIXTURES_DIR, "in-sync")
_STALE = os.path.join(_FIXTURES_DIR, "stale-manifest")
_HAND_EDITED = os.path.join(_FIXTURES_DIR, "hand-edited")


def _run_stage(model_dir, baseline=None):
    """Run stage_docs.py --in <dir> --out <tmp> [--baseline <dir>].

    Returns (returncode, fragment_dict_or_None, stderr).
    """
    if baseline is None:
        baseline = os.path.join(model_dir, "baseline.json")
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
        out_path = tf.name
    try:
        cmd = [
            sys.executable, _STAGE_SCRIPT,
            "--in", model_dir,
            "--out", out_path,
            "--baseline", baseline,
        ]
        result = subprocess.run(cmd, capture_output=True, text=True)
        fragment = None
        if os.path.exists(out_path) and os.path.getsize(out_path) > 0:
            with open(out_path) as f:
                fragment = json.load(f)
        return result.returncode, fragment, result.stderr
    finally:
        if os.path.exists(out_path):
            os.unlink(out_path)


def _finding_codes(fragment):
    return {f.get("code") for f in fragment.get("findings", [])}


class TestFragmentShape(unittest.TestCase):
    _VALID_STATUSES = {"pass", "fail", "warn", "skip"}

    def _assert_valid_fragment(self, fragment, expected_stage="docs"):
        self.assertIsInstance(fragment, dict, "fragment must be a dict")
        self.assertIn("stage", fragment)
        self.assertEqual(fragment["stage"], expected_stage)
        self.assertIn("status", fragment)
        self.assertIn(fragment["status"], self._VALID_STATUSES,
                      f"invalid status: {fragment['status']!r}")
        self.assertIn("detail", fragment)
        self.assertIn("findings", fragment)
        self.assertIsInstance(fragment["findings"], list)

    def test_in_sync_fragment_shape(self):
        _, fragment, _ = _run_stage(_IN_SYNC)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment)

    def test_stale_fragment_shape(self):
        _, fragment, _ = _run_stage(_STALE)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment)

    def test_hand_edited_fragment_shape(self):
        _, fragment, _ = _run_stage(_HAND_EDITED)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment)


class TestInSync(unittest.TestCase):
    """All artifacts match baseline + all docs fresh → pass."""

    def setUp(self):
        self.rc, self.fragment, self.stderr = _run_stage(_IN_SYNC)

    def test_fixture_exists(self):
        self.assertTrue(os.path.isdir(_IN_SYNC))
        self.assertTrue(os.path.exists(os.path.join(_IN_SYNC, "baseline.json")))

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0, f"Expected exit 0; stderr={self.stderr}")

    def test_status_pass(self):
        self.assertEqual(self.fragment["status"], "pass",
                         f"Expected pass; fragment={self.fragment}")

    def test_no_findings(self):
        self.assertEqual(self.fragment["findings"], [],
                         f"Expected no findings; fragment={self.fragment}")


class TestStaleManifest(unittest.TestCase):
    """MANIFEST omits a current model → fail + docs_stale."""

    def setUp(self):
        self.rc, self.fragment, self.stderr = _run_stage(_STALE)

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0, f"Expected exit 0; stderr={self.stderr}")

    def test_status_fail(self):
        self.assertEqual(self.fragment["status"], "fail",
                         f"Expected fail; fragment={self.fragment}")

    def test_finding_docs_stale(self):
        codes = _finding_codes(self.fragment)
        self.assertIn("docs_stale", codes,
                      f"Expected docs_stale; findings={self.fragment['findings']}")

    def test_not_hand_edited(self):
        # Generated artifacts in this fixture still match baseline.
        codes = _finding_codes(self.fragment)
        self.assertNotIn("hand_edited_generated", codes)

    def test_names_stale_doc_and_fact(self):
        # The finding should name the stale doc + missing fact (MANIFEST/dust_deputy).
        blob = json.dumps(self.fragment)
        self.assertIn("MANIFEST", blob)
        self.assertIn("dust_deputy", blob)


class TestHandEdited(unittest.TestCase):
    """A build/ artifact's bytes differ from baseline → fail + hand_edited_generated."""

    def setUp(self):
        self.rc, self.fragment, self.stderr = _run_stage(_HAND_EDITED)

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0, f"Expected exit 0; stderr={self.stderr}")

    def test_status_fail(self):
        self.assertEqual(self.fragment["status"], "fail",
                         f"Expected fail; fragment={self.fragment}")

    def test_finding_hand_edited(self):
        codes = _finding_codes(self.fragment)
        self.assertIn("hand_edited_generated", codes,
                      f"Expected hand_edited_generated; findings={self.fragment['findings']}")

    def test_names_the_file(self):
        # The guard must name the offending generated file.
        blob = json.dumps(self.fragment)
        self.assertIn("cyclone_inlet.stl", blob)


class TestMissingInArg(unittest.TestCase):
    """Omitting --in should exit non-zero (usage error)."""

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


class TestMissingGeneratedArtifact(unittest.TestCase):
    """A baseline-listed generated artifact that is ABSENT on disk (e.g. a
    deleted/never-regenerated file) must trip hand_edited_generated — proving
    the guard fails closed on a missing file, not only on a content mismatch."""

    def test_deleted_generated_artifact_fails(self):
        tmp = tempfile.mkdtemp()
        try:
            model_dir = os.path.join(tmp, "model")
            shutil.copytree(_IN_SYNC, model_dir)
            baseline_path = os.path.join(model_dir, "baseline.json")
            with open(baseline_path) as f:
                baseline = json.load(f)
            # Pick a generated artifact the baseline tracks and delete it.
            tracked = sorted(baseline.keys() if isinstance(baseline, dict)
                             else [e["path"] for e in baseline])
            victim = tracked[0]
            os.unlink(os.path.join(model_dir, victim))

            rc, fragment, _ = _run_stage(model_dir, baseline=baseline_path)
            self.assertEqual(rc, 0, "verdict lives in status, not exit code")
            self.assertEqual(fragment["status"], "fail")
            self.assertIn("hand_edited_generated", _finding_codes(fragment),
                          "a deleted baseline-tracked artifact must fail closed")
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()

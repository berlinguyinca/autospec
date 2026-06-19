"""
test_stage_docs.py — unittest for stage_docs.py.

Real file diffs, no mocks. Drives the stage via its uniform CLI against model
dirs that each test MATERIALIZES AT RUNTIME under a tempfile.mkdtemp(). Nothing
here reads any committed fixture or any committed build/ artifact, so the suite
passes on a clean checkout (where build/ is gitignored and absent).

Each scenario builds a fresh model dir containing:
  - hand docs MANIFEST.md / README.md / FITTINGS_AND_SCREWS.md (inline content)
  - facts.json with two models (cyclone_inlet, dust_deputy)
  - build/ artifacts with deterministic bytes
  - baseline.json = {relpath: sha256(materialized file)} computed in-code, so
    the regen baseline is correct by construction.

Scenarios:
  1. in-sync        → status pass  (artifacts match baseline; docs reference both models).
  2. stale-manifest → status fail + docs_stale (MANIFEST omits dust_deputy);
                      NO hand_edited_generated (artifacts still match baseline).
  3. hand-edited    → status fail + hand_edited_generated (one build artifact
                      mutated after baseline so its sha256 no longer matches).
  4. missing artifact → status fail + hand_edited_generated (a baseline-tracked
                      build artifact deleted → guard fails closed on absence).
  5. fragment shape → required stage-record keys + valid status enum.
  6. exit codes     → verdict in status (exit 0); missing --in is usage error.
"""

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_SCRIPTS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "scripts"))
_STAGE_SCRIPT = os.path.join(_SCRIPTS_DIR, "stage_docs.py")

_MODELS = ("cyclone_inlet", "dust_deputy")

# Deterministic build/ artifacts (relpath -> bytes). Parent dirs created on write.
_BUILD_ARTIFACTS = {
    "build/BOM.md": b"# Bill of Materials\ncyclone_inlet x1\ndust_deputy x1\n",
    "build/datasheet.pdf": b"%PDF-1.4\n%fake-deterministic-pdf-bytes\n%%EOF\n",
    "build/renders/contact_sheet.txt": b"contact sheet: cyclone_inlet, dust_deputy\n",
    "build/stls/manifolds/cyclone_inlet.stl": b"solid cyclone_inlet\nendsolid\n",
    "build/stls/manifolds/dust_deputy.stl": b"solid dust_deputy\nendsolid\n",
}


def _sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def _write_file(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as fh:
        fh.write(data)


def _hand_docs(include_models):
    """Return {filename: text} for the three hand docs.

    Each doc references every model name in `include_models`. Omitting a model
    from MANIFEST (via a smaller include list) makes that doc stale.
    """
    refs = ", ".join(include_models)
    all_refs = ", ".join(_MODELS)
    return {
        "MANIFEST.md": f"# Manifest\nModels: {refs}\n",
        "README.md": f"# Cyclone Kit\nIncludes models: {all_refs}\n",
        "FITTINGS_AND_SCREWS.md": f"# Fittings\nFor models: {all_refs}\n",
    }


def _materialize_model_dir(manifest_models=_MODELS):
    """Build a fresh tmp model dir with hand docs, facts.json, build/ artifacts,
    and a baseline.json correct by construction. Returns the model dir path.

    Caller is responsible for cleanup (shutil.rmtree of the parent tmp).
    """
    model_dir = tempfile.mkdtemp(prefix="stage_docs_")

    # Hand docs (MANIFEST may omit models when manifest_models is shorter).
    docs = _hand_docs(manifest_models)
    for name, text in docs.items():
        _write_file(os.path.join(model_dir, name), text.encode("utf-8"))

    # facts.json — current model list.
    _write_file(
        os.path.join(model_dir, "facts.json"),
        json.dumps({"models": list(_MODELS)}).encode("utf-8"),
    )

    # build/ artifacts + baseline (sha256 by construction).
    baseline = {}
    for rel, data in _BUILD_ARTIFACTS.items():
        _write_file(os.path.join(model_dir, rel), data)
        baseline[rel] = _sha256_bytes(data)
    _write_file(
        os.path.join(model_dir, "baseline.json"),
        json.dumps(baseline).encode("utf-8"),
    )
    return model_dir


def _run_stage(model_dir, baseline=None):
    """Run stage_docs.py --in <dir> --out <tmp> --baseline <baseline>.

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


class _MaterializedModelMixin:
    """Provides a self.model_dir materialized in setUp and cleaned up after."""

    manifest_models = _MODELS

    def setUp(self):
        self.model_dir = _materialize_model_dir(self.manifest_models)
        self.addCleanup(shutil.rmtree, self.model_dir, ignore_errors=True)


class TestFragmentShape(_MaterializedModelMixin, unittest.TestCase):
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
        _, fragment, _ = _run_stage(self.model_dir)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment)

    def test_stale_fragment_shape(self):
        # Mutate MANIFEST to omit a model, then run.
        _write_file(
            os.path.join(self.model_dir, "MANIFEST.md"),
            b"# Manifest\nModels: cyclone_inlet\n",
        )
        _, fragment, _ = _run_stage(self.model_dir)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment)

    def test_hand_edited_fragment_shape(self):
        # Mutate a build artifact so it diverges from baseline, then run.
        victim = os.path.join(self.model_dir,
                              "build/stls/manifolds/cyclone_inlet.stl")
        with open(victim, "ab") as fh:
            fh.write(b"hand edit\n")
        _, fragment, _ = _run_stage(self.model_dir)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment)


class TestInSync(_MaterializedModelMixin, unittest.TestCase):
    """All artifacts match baseline + all docs fresh → pass."""

    def setUp(self):
        super().setUp()
        self.rc, self.fragment, self.stderr = _run_stage(self.model_dir)

    def test_baseline_materialized(self):
        self.assertTrue(os.path.exists(
            os.path.join(self.model_dir, "baseline.json")))
        # build/ artifacts exist on disk (materialized, not committed).
        for rel in _BUILD_ARTIFACTS:
            self.assertTrue(os.path.exists(os.path.join(self.model_dir, rel)))

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0, f"Expected exit 0; stderr={self.stderr}")

    def test_status_pass(self):
        self.assertEqual(self.fragment["status"], "pass",
                         f"Expected pass; fragment={self.fragment}")

    def test_no_findings(self):
        self.assertEqual(self.fragment["findings"], [],
                         f"Expected no findings; fragment={self.fragment}")


class TestStaleManifest(_MaterializedModelMixin, unittest.TestCase):
    """MANIFEST omits a current model → fail + docs_stale (artifacts still match)."""

    # MANIFEST references only cyclone_inlet → omits dust_deputy.
    manifest_models = ("cyclone_inlet",)

    def setUp(self):
        super().setUp()
        self.rc, self.fragment, self.stderr = _run_stage(self.model_dir)

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
        # Generated artifacts match the runtime baseline by construction.
        codes = _finding_codes(self.fragment)
        self.assertNotIn("hand_edited_generated", codes)

    def test_names_stale_doc_and_fact(self):
        # The finding should name the stale doc + missing fact.
        blob = json.dumps(self.fragment)
        self.assertIn("MANIFEST", blob)
        self.assertIn("dust_deputy", blob)


class TestHandEdited(_MaterializedModelMixin, unittest.TestCase):
    """A build/ artifact's bytes differ from baseline → fail + hand_edited_generated."""

    def setUp(self):
        super().setUp()
        # After baseline is written, mutate one build artifact's bytes.
        victim = os.path.join(self.model_dir,
                              "build/stls/manifolds/cyclone_inlet.stl")
        with open(victim, "ab") as fh:
            fh.write(b"hand edit\n")
        self.rc, self.fragment, self.stderr = _run_stage(self.model_dir)

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


class TestMissingGeneratedArtifact(_MaterializedModelMixin, unittest.TestCase):
    """A baseline-listed generated artifact that is ABSENT on disk must trip
    hand_edited_generated — proving the guard fails closed on a missing file,
    not only on a content mismatch."""

    def test_deleted_generated_artifact_fails(self):
        baseline_path = os.path.join(self.model_dir, "baseline.json")
        with open(baseline_path) as f:
            baseline = json.load(f)
        victim = sorted(baseline.keys())[0]
        os.unlink(os.path.join(self.model_dir, victim))

        rc, fragment, _ = _run_stage(self.model_dir, baseline=baseline_path)
        self.assertEqual(rc, 0, "verdict lives in status, not exit code")
        self.assertEqual(fragment["status"], "fail")
        self.assertIn("hand_edited_generated", _finding_codes(fragment),
                      "a deleted baseline-tracked artifact must fail closed")


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


class TestNoDocsContextSkips(unittest.TestCase):
    """The release-gate engine sequences stages with --in <STL file> (not a
    model directory) and no docs context. The docs stage must DEGRADE to a
    non-blocking skip — there is nothing to drift against — rather than
    hard-fail, or a circuit/duct-less model could never reach a green gate.
    """

    def test_in_is_a_file_skips(self):
        # --in points at a regular file, not a model dir (engine's STL path).
        with tempfile.NamedTemporaryFile(suffix=".stl", delete=False) as tf:
            tf.write(b"solid x\nendsolid x\n")
            stl_path = tf.name
        try:
            rc, fragment, stderr = _run_stage(stl_path)
            self.assertEqual(rc, 0, f"verdict in status, not exit code: {stderr}")
            self.assertIsNotNone(fragment)
            self.assertEqual(fragment["status"], "skip")
            self.assertEqual(fragment["findings"], [])
        finally:
            os.unlink(stl_path)

    def test_dir_without_facts_skips(self):
        # A model dir with no facts.json: no docs context to verify → skip.
        model_dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, model_dir, ignore_errors=True)
        rc, fragment, stderr = _run_stage(model_dir)
        self.assertEqual(rc, 0, f"verdict in status, not exit code: {stderr}")
        self.assertIsNotNone(fragment)
        self.assertEqual(fragment["status"], "skip")


if __name__ == "__main__":
    unittest.main()

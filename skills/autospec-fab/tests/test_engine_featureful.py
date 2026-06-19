"""test_engine_featureful.py — engine threads per-model stage inputs (D2/D3).

Phase 5.5 integration audit found that stl-release-gate.py invoked every stage
with only `--in <stl> --model <metadata> --out`, so the vacuum-circuit and
dust-airflow stages never received their stage-specific descriptor and always
SKIPPED — a featureful model passed GREEN without those gates ever running
(false assurance).

The fix: the engine resolves per-model stage-specific inputs by a SIBLING-FILE
convention relative to the model sidecar's directory (MODELDIR = dirname of
--model) and threads them when the file exists:

  vacuum-circuit  MODELDIR/circuit.json  -> --circuit
  dust-airflow    MODELDIR/duct.json     -> --duct
  fea             MODELDIR/load.json     -> --load
  cfd             MODELDIR/flow.json     -> --flow
  docs            --in MODELDIR (the dir, not the stl) [+ baseline.json]

These tests run the REAL engine over the committed featureful dogfood fixture
(no mocks) and assert the gates actually RAN (status pass, not skip). They also
prove the threaded gate genuinely BITES: swapping in a broken circuit makes the
gate go RED. The original featureless dogfood must still go green (circuit/dust
skip — correct for a model with no sibling descriptors).

Stdlib only; runs on bare python3.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPTS_DIR = os.path.abspath(os.path.join(HERE, "..", "scripts"))
FIXTURES = os.path.abspath(os.path.join(HERE, "..", "fixtures"))
ENGINE = os.path.join(SCRIPTS_DIR, "stl-release-gate.py")
FEATUREFUL = os.path.join(FIXTURES, "dogfood-featureful")
FEATURELESS = os.path.join(FIXTURES, "dogfood")
BROKEN_CIRCUIT = os.path.join(
    HERE, "fixtures", "circuit", "broken-link.json")


def _run_engine(model_dir, out_path):
    """Run the real engine over <model_dir>/{dogfood.stl,metadata.json}."""
    argv = [
        sys.executable, ENGINE,
        "--in", os.path.join(model_dir, "dogfood.stl"),
        "--model", os.path.join(model_dir, "metadata.json"),
        "--out", out_path,
    ]
    return subprocess.run(argv, capture_output=True, text=True)


def _statuses(out_path):
    with open(out_path, encoding="utf-8") as f:
        gate = json.load(f)
    return {s["stage"]: s["status"] for s in gate["stages"]}, gate


class EngineFeaturefulTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="fab-featureful-")
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.out = os.path.join(self.tmp, "release-gate.json")

    # ---- D3: featureful dogfood -> green, circuit+dust RAN (pass) --------
    def test_featureful_green_and_gates_ran(self):
        proc = _run_engine(FEATUREFUL, self.out)
        self.assertEqual(proc.returncode, 0,
                         "featureful dogfood must be GREEN; stderr=%s"
                         % proc.stderr)
        statuses, gate = _statuses(self.out)
        # The whole point: these gates must RUN (pass), not skip.
        self.assertEqual(statuses["vacuum-circuit"], "pass",
                         "engine must thread circuit.json -> circuit gate ran")
        self.assertEqual(statuses["dust-airflow"], "pass",
                         "engine must thread duct.json -> dust gate ran")
        # D1: metadata sidecar carries name + body_count and must validate.
        self.assertEqual(statuses["metadata"], "pass",
                         "metadata gate must pass (schema accepts name+body_count)")
        self.assertEqual(gate["name"], "DogfoodFeaturefulManifold")

    # ---- D2 bite: a BAD circuit.json drives the gate RED via the engine --
    def test_featureful_bad_circuit_goes_red(self):
        # Copy the featureful fixture, swap in a broken circuit graph.
        staged = os.path.join(self.tmp, "bad-model")
        shutil.copytree(FEATUREFUL, staged)
        shutil.copyfile(BROKEN_CIRCUIT, os.path.join(staged, "circuit.json"))
        proc = _run_engine(staged, self.out)
        self.assertNotEqual(proc.returncode, 0,
                            "a broken circuit.json must drive the gate RED "
                            "(proves the threaded gate genuinely bites)")
        statuses, _ = _statuses(self.out)
        self.assertEqual(statuses["vacuum-circuit"], "fail")

    # ---- featureless dogfood still green; circuit/dust skip (no siblings)-
    def test_featureless_still_green_skips(self):
        proc = _run_engine(FEATURELESS, self.out)
        self.assertEqual(proc.returncode, 0,
                         "featureless dogfood must remain GREEN; stderr=%s"
                         % proc.stderr)
        statuses, _ = _statuses(self.out)
        self.assertEqual(statuses["vacuum-circuit"], "skip",
                         "no circuit.json sibling -> circuit gate skips")
        self.assertEqual(statuses["dust-airflow"], "skip",
                         "no duct.json sibling -> dust gate skips")


if __name__ == "__main__":
    unittest.main()

"""test_release_gate_results.py — engine aggregation of fea_results/cfd_results.

The release-gate engine extracts the structured `fea_results` from the fea
stage fragment and `cfd_results` from the cfd stage fragment (mirroring the
existing vision_findings extraction) and places them at the TOP LEVEL of the
written gate. The per-stage stages[] records stay projected to
{stage,status,detail,findings}; the structured results live at gate top-level.

Stage fragments are stubbed via test_release_gate's stub harness, which emits
fea_results/cfd_results from AUTOSPEC_STUB_FEA_RESULTS_FEA /
AUTOSPEC_STUB_CFD_RESULTS_CFD. Schema validation uses the real `ajv` CLI when
present (guarded by shutil.which).
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_release_gate import (  # noqa: E402
    ENGINE, SCHEMA, _build_stub_dir, _env_for)


class ReleaseGateResultsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="relgate-results-")
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.stl = os.path.join(self.tmp, "model.stl")
        with open(self.stl, "w", encoding="utf-8") as f:
            f.write("solid m\nendsolid m\n")
        self.model = os.path.join(self.tmp, "model.json")
        with open(self.model, "w", encoding="utf-8") as f:
            json.dump({"name": "DogfoodManifold"}, f)
        self.out = os.path.join(self.tmp, "release-gate.json")
        self.stages_dir = _build_stub_dir(self.tmp)

    def _run(self, env):
        argv = [
            sys.executable, ENGINE,
            "--in", self.stl,
            "--model", self.model,
            "--out", self.out,
            "--stages-dir", self.stages_dir,
        ]
        return subprocess.run(argv, env=env, capture_output=True, text=True)

    def _read_gate(self):
        with open(self.out, encoding="utf-8") as f:
            return json.load(f)

    def _ajv_validate(self):
        return subprocess.run(
            ["ajv", "validate", "-s", SCHEMA, "--spec=draft2020", "-d", self.out],
            capture_output=True, text=True,
        )

    def _env_with_results(self):
        env = _env_for()
        env["AUTOSPEC_STUB_FEA_RESULTS_FEA"] = json.dumps(
            {"safety_factor": 3.5, "required_min": 2.0, "status": "pass"})
        env["AUTOSPEC_STUB_CFD_RESULTS_CFD"] = json.dumps(
            {"pressure_drop_pa": 200.0, "min_velocity_m_s": 3.0,
             "stagnation": False, "status": "pass"})
        return env

    def test_results_aggregated_to_top_level(self):
        proc = self._run(self._env_with_results())
        self.assertEqual(proc.returncode, 0,
                         "green gate expected; stderr=%s" % proc.stderr)
        gate = self._read_gate()
        self.assertEqual(
            gate["fea_results"],
            {"safety_factor": 3.5, "required_min": 2.0, "status": "pass"})
        self.assertEqual(
            gate["cfd_results"],
            {"pressure_drop_pa": 200.0, "min_velocity_m_s": 3.0,
             "stagnation": False, "status": "pass"})

    def test_results_not_in_stage_records(self):
        self._run(self._env_with_results())
        gate = self._read_gate()
        for s in gate["stages"]:
            self.assertEqual(set(s.keys()),
                             {"stage", "status", "detail", "findings"})

    def test_aggregated_gate_is_schema_valid(self):
        self._run(self._env_with_results())
        if not shutil.which("ajv"):
            self.skipTest("ajv not on PATH")
        v = self._ajv_validate()
        self.assertEqual(v.returncode, 0,
                         "gate must be schema-valid; ajv: %s %s"
                         % (v.stdout, v.stderr))

    def test_absent_results_omitted_from_gate(self):
        """fea/cfd skip (no results emitted) -> no top-level keys, still valid."""
        proc = self._run(_env_for())
        self.assertEqual(proc.returncode, 0)
        gate = self._read_gate()
        self.assertNotIn("fea_results", gate)
        self.assertNotIn("cfd_results", gate)
        if shutil.which("ajv"):
            self.assertEqual(self._ajv_validate().returncode, 0)


if __name__ == "__main__":
    unittest.main()

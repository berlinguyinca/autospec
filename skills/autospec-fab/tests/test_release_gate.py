"""test_release_gate.py — unittest suite for the stl-release-gate.py ENGINE.

The engine (skills/autospec-fab/scripts/stl-release-gate.py) sequences the 12
fab stages, aggregates their per-stage fragments into a schema-valid
.autospec/fab/release-gate.json, and decides the verdict:
  - any BLOCKING stage fragment with status "fail" -> non-zero exit;
  - the vision stage is ADVISORY: its warn/fail never blocks, its observations
    go to release-gate.json["vision_findings"].

Stages are stubbed via a --stages-dir of tiny python stub scripts whose
emitted status / detail / findings / vision_findings are driven by environment
variables, so each test can shape the per-stage verdict deterministically.

Runs on bare python3 (stdlib only). Schema validation uses the real `ajv` CLI
when present (assertions guarded by command -v ajv / shutil.which).
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
sys.path.insert(0, SCRIPTS_DIR)
from release_gate_stages import STAGE_ORDER  # noqa: E402  (engine's own table)
REPO_ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
ENGINE = os.path.join(SCRIPTS_DIR, "stl-release-gate.py")
SCHEMA = os.path.join(REPO_ROOT, "schemas", "autospec-fab-release-gate.schema.json")

# The 12 stage names in composition order; must match the engine's STAGE_ORDER.
STAGE_NAMES = [
    "geometry", "metadata", "vacuum-fitting", "vacuum-circuit", "gasket",
    "dust-airflow", "slicer", "fea", "cfd", "render", "vision", "docs",
]

# A stub stage script. It writes a release-gate fragment to --out. Its status,
# detail, findings, and vision_findings are read from per-stage env vars so the
# test harness can shape each stage's verdict:
#   AUTOSPEC_STUB_STATUS_<stage>          -> status (default "pass")
#   AUTOSPEC_STUB_FINDINGS_<stage>        -> JSON array for findings (default [])
#   AUTOSPEC_STUB_VISION_<stage>          -> JSON array for vision_findings
# stage names are uppercased and non-alnum -> "_" for the env var suffix.
STUB_SOURCE = r'''#!/usr/bin/env python3
import argparse, json, os, sys

# This stub derives WHICH stage it is from its own filename (argv[0]) using the
# engine's own script->stage-name map, injected as SCRIPT_TO_STAGE below. That
# lets one stub body serve all 12 stages; the engine picks which file to run.
SCRIPT_TO_STAGE = __SCRIPT_TO_STAGE__

def _stage_from_argv0():
    base = os.path.basename(sys.argv[0])
    return SCRIPT_TO_STAGE.get(base, base)

def main():
    p = argparse.ArgumentParser()
    p.add_argument("--in", dest="in_path", required=True)
    p.add_argument("--model", dest="model_path", default=None)
    p.add_argument("--out", required=True)
    # tolerate any extra stage-specific flags the engine might pass
    args, _unknown = p.parse_known_args()
    stage = _stage_from_argv0()
    key = "".join(c if c.isalnum() else "_" for c in stage).upper()
    status = os.environ.get("AUTOSPEC_STUB_STATUS_" + key, "pass")
    findings = json.loads(os.environ.get("AUTOSPEC_STUB_FINDINGS_" + key, "[]"))
    fragment = {
        "stage": stage,
        "status": status,
        "detail": stage + " stub: " + status,
        "findings": findings,
    }
    vision = os.environ.get("AUTOSPEC_STUB_VISION_" + key)
    if vision is not None:
        fragment["vision_findings"] = json.loads(vision)
    # exercise the "engine strips non-schema keys" path
    fragment["_stub_extra"] = "should-be-dropped-by-engine"
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(fragment, f)
    sys.exit(0)

if __name__ == "__main__":
    main()
'''


def _build_stub_dir(base):
    """Create a --stages-dir of stub stage scripts mirroring the real names."""
    sdir = os.path.join(base, "stages")
    os.makedirs(sdir, exist_ok=True)
    # Build the canonical script-filename -> stage-name map from the engine's
    # own STAGE_ORDER (plus the alternate hyphen/underscore filename variant),
    # so each stub knows which stage it stands in for regardless of which
    # filename the engine resolves. A single stub body then serves all stages.
    script_to_stage = {}
    for st in STAGE_ORDER:
        script = st["script"]
        script_to_stage[script] = st["name"]
        if script.startswith("stage-"):
            alt = "stage_" + script[len("stage-"):].replace("-", "_")
        else:
            alt = "stage-" + script[len("stage_"):].replace("_", "-")
        script_to_stage[alt] = st["name"]
    body = STUB_SOURCE.replace("__SCRIPT_TO_STAGE__", repr(script_to_stage))
    for fname in script_to_stage:
        path = os.path.join(sdir, fname)
        with open(path, "w", encoding="utf-8") as f:
            f.write(body)
        os.chmod(path, 0o755)
    return sdir


def _env_for(status_overrides=None, findings_overrides=None, vision_overrides=None):
    env = dict(os.environ)
    status_overrides = status_overrides or {}
    findings_overrides = findings_overrides or {}
    vision_overrides = vision_overrides or {}
    for stage in STAGE_NAMES:
        key = "".join(c if c.isalnum() else "_" for c in stage).upper()
        env["AUTOSPEC_STUB_STATUS_" + key] = status_overrides.get(stage, "pass")
        if stage in findings_overrides:
            env["AUTOSPEC_STUB_FINDINGS_" + key] = json.dumps(findings_overrides[stage])
        if stage in vision_overrides:
            env["AUTOSPEC_STUB_VISION_" + key] = json.dumps(vision_overrides[stage])
    return env


class ReleaseGateEngineTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="relgate-")
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.stl = os.path.join(self.tmp, "model.stl")
        with open(self.stl, "w", encoding="utf-8") as f:
            f.write("solid m\nendsolid m\n")
        self.model = os.path.join(self.tmp, "model.json")
        with open(self.model, "w", encoding="utf-8") as f:
            json.dump({"name": "DogfoodManifold"}, f)
        self.out = os.path.join(self.tmp, "release-gate.json")
        self.stages_dir = _build_stub_dir(self.tmp)

    def _run(self, env, extra_args=None):
        argv = [
            sys.executable, ENGINE,
            "--in", self.stl,
            "--model", self.model,
            "--out", self.out,
            "--stages-dir", self.stages_dir,
        ]
        if extra_args:
            argv += extra_args
        return subprocess.run(argv, env=env, capture_output=True, text=True)

    def _read_gate(self):
        with open(self.out, encoding="utf-8") as f:
            return json.load(f)

    def _ajv_validate(self):
        proc = subprocess.run(
            ["ajv", "validate", "-s", SCHEMA, "--spec=draft2020", "-d", self.out],
            capture_output=True, text=True,
        )
        return proc

    # ---- all-pass: schema-valid green gate, exit 0 -----------------------
    def test_all_pass_green_gate(self):
        proc = self._run(_env_for())
        self.assertEqual(proc.returncode, 0,
                         "all-pass should exit 0; stderr=%s" % proc.stderr)
        self.assertTrue(os.path.exists(self.out))
        gate = self._read_gate()
        self.assertEqual(gate["schema_version"], 1)
        self.assertEqual(gate["name"], "DogfoodManifold")
        self.assertIn("geometry_hash", gate)
        self.assertEqual([s["stage"] for s in gate["stages"]], STAGE_NAMES)
        for s in gate["stages"]:
            self.assertEqual(s["status"], "pass")
            self.assertEqual(set(s.keys()), {"stage", "status", "detail", "findings"})
            self.assertNotIn("_stub_extra", s)
        if shutil.which("ajv"):
            v = self._ajv_validate()
            self.assertEqual(v.returncode, 0,
                             "gate must be schema-valid; ajv: %s %s" % (v.stdout, v.stderr))

    # ---- one hard-reject: blocking fail -> non-zero exit -----------------
    def test_one_hard_reject_nonzero(self):
        env = _env_for(
            status_overrides={"geometry": "fail"},
            findings_overrides={"geometry": [{"code": "non_watertight"}]},
        )
        proc = self._run(env)
        self.assertNotEqual(proc.returncode, 0,
                            "blocking fail must exit non-zero")
        gate = self._read_gate()
        geo = next(s for s in gate["stages"] if s["stage"] == "geometry")
        self.assertEqual(geo["status"], "fail")
        self.assertEqual(geo["findings"], [{"code": "non_watertight"}])
        if shutil.which("ajv"):
            self.assertEqual(self._ajv_validate().returncode, 0)

    # ---- vision is advisory: warn never blocks ---------------------------
    def test_vision_non_blocking(self):
        env = _env_for(
            status_overrides={"vision": "warn"},
            vision_overrides={"vision": [{"code": "blocked_npt", "note": "looks blocked"}]},
        )
        proc = self._run(env)
        self.assertEqual(proc.returncode, 0,
                         "vision warn must NOT block; stderr=%s" % proc.stderr)
        gate = self._read_gate()
        vis = next(s for s in gate["stages"] if s["stage"] == "vision")
        self.assertEqual(vis["status"], "warn")
        self.assertEqual(
            gate["vision_findings"],
            [{"code": "blocked_npt", "note": "looks blocked"}],
        )
        if shutil.which("ajv"):
            self.assertEqual(self._ajv_validate().returncode, 0)

    # ---- vision fail also does not block ---------------------------------
    def test_vision_fail_still_green(self):
        env = _env_for(status_overrides={"vision": "fail"})
        proc = self._run(env)
        self.assertEqual(proc.returncode, 0,
                         "even vision 'fail' is advisory and must not block")

    # ---- skip does not fail ----------------------------------------------
    def test_skip_does_not_fail(self):
        env = _env_for(status_overrides={"fea": "skip", "cfd": "skip"})
        proc = self._run(env)
        self.assertEqual(proc.returncode, 0)

    # ---- stage order + shape preserved -----------------------------------
    def test_stage_order_and_shape(self):
        proc = self._run(_env_for())
        self.assertEqual(proc.returncode, 0)
        gate = self._read_gate()
        self.assertEqual([s["stage"] for s in gate["stages"]], STAGE_NAMES)


if __name__ == "__main__":
    unittest.main()

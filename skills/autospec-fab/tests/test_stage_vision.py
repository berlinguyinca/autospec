"""
test_stage_vision.py — unittest for stage_vision.py (advisory vision stage).

Strategy: prepend a $TMP/bin directory containing a fake vision CLI shim to
PATH (and point $AUTOSPEC_FAB_VISION_CMD at it). The shim emits a known
advisory observation on its first ("judge") pass and a confirm/reject verdict
on its second ("--verify") pass — both env-controllable:

  VISION_OBS       observation text the judge pass emits (default a generic one)
  VISION_SEV       severity of that observation: "info" | "warn" (default warn)
  VISION_VIEW      optional view label
  VISION_RULE      optional rule id (e.g. exposed_gasket)
  VISION_CONFIRM   "1" (default) the verify pass confirms; "0" it rejects

No real vision model is needed. Pure stdlib + the PATH/env shim.

Assertions:
  (a) the shim's finding is RECORDED in vision_findings.
  (b) the stage stays NON-BLOCKING: even when the shim emits an alarming /
      "reject"-flavoured observation (a hard-reject code at warn severity) the
      status is NEVER "fail" — at most "warn".
  (c) vision absent -> status "skip", empty vision_findings.
  (d) the observation lands in vision_findings, NOT the blocking findings[].
  (e) fragment shape: {stage, status, detail, findings, vision_findings}.
  (f) the adversarial verify pass actually filters: a rejected observation is
      dropped from vision_findings.
"""

import json
import os
import shutil
import stat
import sys
import tempfile
import unittest

_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_SCRIPTS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "scripts"))
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

import stage_vision  # noqa: E402

# A vision CLI shim with two passes:
#   judge pass:  argv = [shim, <sheet>, <rules>]        -> emits observations
#   verify pass: argv = [shim, "--verify", <sheet>, <rules>] with the candidate
#                observation JSON on stdin -> emits {"confirmed": bool}
# This mirrors the adversarial second-pass: every observation must be
# re-confirmed before it is recorded.
_VISION_SHIM = r'''#!/usr/bin/env python3
import json, os, sys

obs_text = os.environ.get("VISION_OBS", "surface looks acceptable")
sev = os.environ.get("VISION_SEV", "warn")
view = os.environ.get("VISION_VIEW", "")
rule = os.environ.get("VISION_RULE", "")
confirm = os.environ.get("VISION_CONFIRM", "1") == "1"

if len(sys.argv) >= 2 and sys.argv[1] == "--verify":
    # second pass: confirm or reject the candidate observation.
    try:
        json.load(sys.stdin)
    except Exception:
        pass
    json.dump({"confirmed": confirm}, sys.stdout)
    sys.exit(0)

ob = {"observation": obs_text, "severity": sev}
if view:
    ob["view"] = view
if rule:
    ob["rule"] = rule
json.dump({"observations": [ob]}, sys.stdout)
sys.exit(0)
'''


def _install_vision_shim(tmp_bin):
    dst = os.path.join(tmp_bin, "fab-vision")
    with open(dst, "w") as f:
        f.write(_VISION_SHIM)
    mode = os.stat(dst).st_mode
    os.chmod(dst, mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return dst


class _VisionBase(unittest.TestCase):

    def setUp(self):
        self._orig_path = os.environ.get("PATH", "")
        self._orig_cmd = os.environ.get("AUTOSPEC_FAB_VISION_CMD")
        self._tmpdir = tempfile.mkdtemp(prefix="vision_test_")
        self._tmp_bin = os.path.join(self._tmpdir, "bin")
        os.makedirs(self._tmp_bin)
        shim = _install_vision_shim(self._tmp_bin)
        os.environ["PATH"] = self._tmp_bin + os.pathsep + self._orig_path
        os.environ["AUTOSPEC_FAB_VISION_CMD"] = shim
        for k in ("VISION_OBS", "VISION_SEV", "VISION_VIEW",
                  "VISION_RULE", "VISION_CONFIRM"):
            os.environ.pop(k, None)
        # A stand-in contact sheet; the shim never reads it.
        self._render_dir = os.path.join(self._tmpdir, "renders", "widget")
        os.makedirs(self._render_dir)
        self._sheet = os.path.join(self._render_dir, "contact-sheet.html")
        with open(self._sheet, "w") as f:
            f.write("<html><body>contact sheet</body></html>\n")
        self._rules = os.path.join(self._tmpdir, "rules.md")
        with open(self._rules, "w") as f:
            f.write("# STL Modeling Rules\n- exposed_gasket bad\n")
        self._out = os.path.join(self._tmpdir, "fragment.json")

    def tearDown(self):
        os.environ["PATH"] = self._orig_path
        if self._orig_cmd is None:
            os.environ.pop("AUTOSPEC_FAB_VISION_CMD", None)
        else:
            os.environ["AUTOSPEC_FAB_VISION_CMD"] = self._orig_cmd
        for k in ("VISION_OBS", "VISION_SEV", "VISION_VIEW",
                  "VISION_RULE", "VISION_CONFIRM"):
            os.environ.pop(k, None)
        shutil.rmtree(self._tmpdir, ignore_errors=True)

    def _run(self, in_path=None):
        argv = [
            "--in", in_path or self._sheet,
            "--out", self._out,
            "--rules", self._rules,
        ]
        rc = stage_vision.main(argv)
        with open(self._out) as f:
            frag = json.load(f)
        return rc, frag


class TestFindingRecorded(_VisionBase):

    def test_observation_recorded_in_vision_findings(self):
        os.environ["VISION_OBS"] = "left gasket groove appears exposed"
        os.environ["VISION_SEV"] = "warn"
        os.environ["VISION_VIEW"] = "left"
        os.environ["VISION_RULE"] = "exposed_gasket"
        rc, frag = self._run()
        self.assertEqual(rc, 0)
        vf = frag["vision_findings"]
        self.assertEqual(len(vf), 1)
        self.assertEqual(vf[0]["observation"],
                         "left gasket groove appears exposed")
        self.assertEqual(vf[0]["severity"], "warn")
        self.assertEqual(vf[0]["view"], "left")
        self.assertEqual(vf[0]["rule"], "exposed_gasket")

    def test_warn_observation_sets_warn_status(self):
        os.environ["VISION_SEV"] = "warn"
        _, frag = self._run()
        self.assertEqual(frag["status"], "warn")

    def test_info_only_observation_passes(self):
        os.environ["VISION_SEV"] = "info"
        _, frag = self._run()
        self.assertEqual(frag["status"], "pass")
        self.assertEqual(len(frag["vision_findings"]), 1)


class TestNonBlocking(_VisionBase):

    def test_never_fails_even_on_alarming_observation(self):
        # An alarming, hard-reject-flavoured observation at warn severity.
        os.environ["VISION_OBS"] = ("REJECT: NPT tap is completely blocked, "
                                    "release must not ship")
        os.environ["VISION_SEV"] = "warn"
        os.environ["VISION_RULE"] = "blocked_npt"
        _, frag = self._run()
        # advisory: at most warn, NEVER fail.
        self.assertNotEqual(frag["status"], "fail")
        self.assertEqual(frag["status"], "warn")

    def test_blocking_findings_list_stays_empty(self):
        # Even an alarming observation must NOT enter the blocking findings[].
        os.environ["VISION_OBS"] = "disconnected body visible"
        os.environ["VISION_SEV"] = "warn"
        os.environ["VISION_RULE"] = "disconnected_bodies"
        _, frag = self._run()
        self.assertEqual(frag["findings"], [])
        # ...it lands in the advisory list instead.
        rules = [v.get("rule") for v in frag["vision_findings"]]
        self.assertIn("disconnected_bodies", rules)

    def test_status_is_never_fail_for_any_severity(self):
        for sev in ("info", "warn"):
            with self.subTest(sev=sev):
                os.environ["VISION_SEV"] = sev
                _, frag = self._run()
                self.assertNotEqual(frag["status"], "fail")


class TestAdversarialVerify(_VisionBase):

    def test_rejected_observation_is_dropped(self):
        os.environ["VISION_OBS"] = "hallucinated overhang"
        os.environ["VISION_SEV"] = "warn"
        os.environ["VISION_CONFIRM"] = "0"  # verify pass rejects it
        _, frag = self._run()
        self.assertEqual(frag["vision_findings"], [])
        # nothing confirmed -> nothing to warn about.
        self.assertEqual(frag["status"], "pass")

    def test_confirmed_observation_is_kept(self):
        os.environ["VISION_OBS"] = "real overhang"
        os.environ["VISION_SEV"] = "warn"
        os.environ["VISION_CONFIRM"] = "1"
        _, frag = self._run()
        self.assertEqual(len(frag["vision_findings"]), 1)


class TestVisionAbsent(unittest.TestCase):

    def test_skip_when_vision_cmd_absent(self):
        orig_path = os.environ.get("PATH", "")
        orig_cmd = os.environ.get("AUTOSPEC_FAB_VISION_CMD")
        tmpdir = tempfile.mkdtemp(prefix="vision_absent_")
        try:
            os.environ["PATH"] = "/nonexistent-bin-dir"
            os.environ.pop("AUTOSPEC_FAB_VISION_CMD", None)
            sheet = os.path.join(tmpdir, "contact-sheet.html")
            with open(sheet, "w") as f:
                f.write("<html></html>\n")
            out = os.path.join(tmpdir, "frag.json")
            rc = stage_vision.main(["--in", sheet, "--out", out])
            self.assertEqual(rc, 0)
            with open(out) as f:
                frag = json.load(f)
            self.assertEqual(frag["status"], "skip")
            self.assertEqual(frag["vision_findings"], [])
            self.assertIn("deferred", frag["detail"])
        finally:
            os.environ["PATH"] = orig_path
            if orig_cmd is not None:
                os.environ["AUTOSPEC_FAB_VISION_CMD"] = orig_cmd
            shutil.rmtree(tmpdir, ignore_errors=True)


class TestInputResolution(_VisionBase):

    def test_render_dir_input_resolves_to_contact_sheet(self):
        # --in may be the render dir; the stage finds contact-sheet.html.
        os.environ["VISION_SEV"] = "info"
        rc, frag = self._run(in_path=self._render_dir)
        self.assertEqual(rc, 0)
        self.assertEqual(frag["status"], "pass")

    def test_missing_contact_sheet_skips(self):
        os.environ["VISION_SEV"] = "warn"
        empty = os.path.join(self._tmpdir, "empty")
        os.makedirs(empty)
        rc, frag = self._run(in_path=empty)
        self.assertEqual(rc, 0)
        self.assertEqual(frag["status"], "skip")


class TestFragmentShape(_VisionBase):

    def test_fragment_keys(self):
        os.environ["VISION_SEV"] = "warn"
        _, frag = self._run()
        self.assertEqual(frag["stage"], "vision")
        self.assertIn(frag["status"], ("pass", "warn", "skip"))
        self.assertNotEqual(frag["status"], "fail")
        self.assertIsInstance(frag["detail"], str)
        self.assertIsInstance(frag["findings"], list)
        self.assertIsInstance(frag["vision_findings"], list)


if __name__ == "__main__":
    unittest.main()

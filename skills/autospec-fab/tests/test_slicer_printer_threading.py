"""test_slicer_printer_threading.py — engine threads the printer profile.

The release-gate engine resolves a per-model printer profile by the same
sibling-file convention as circuit/duct/load/flow: when a `printer.json` lives
next to the `--model` sidecar, the engine threads `--printer MODELDIR/printer.json`
into the slicer stage so wall/overhang thresholds reflect the real printer.
Absent → no flag → the slicer keeps its built-in default (no behavior change
for featureless runs). This mirrors how `load.json` → `--load` is wired for fea.

Reuses the engine harness in test_release_gate.ReleaseGateEngineTest (stub
stages-dir, `_run`, `_write_stage_stub`, `_read`).
"""

import json
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPTS_DIR = os.path.abspath(os.path.join(HERE, "..", "scripts"))
sys.path.insert(0, SCRIPTS_DIR)
sys.path.insert(0, HERE)
from release_gate_stages import extra_args_for  # noqa: E402
from test_release_gate import ReleaseGateEngineTest, _env_for  # noqa: E402


class SlicerPrinterThreadingTest(ReleaseGateEngineTest):
    """Inherits setUp/_run/_write_stage_stub/_read from the engine test."""

    def _install_slicer_printer_probe(self):
        """Replace the slicer stub with one recording the --printer it got."""
        self.printer_probe = os.path.join(self.tmp, "slicer-printer.txt")
        slicer_src = (
            "#!/usr/bin/env python3\n"
            "import argparse, json, sys\n"
            "p = argparse.ArgumentParser()\n"
            "p.add_argument('--in', dest='in_path', required=True)\n"
            "p.add_argument('--model', dest='model_path', default=None)\n"
            "p.add_argument('--out', required=True)\n"
            "p.add_argument('--printer', dest='printer_path', default=None)\n"
            "a, _ = p.parse_known_args()\n"
            "open(%r, 'w').write(a.printer_path or '<none>')\n"
            "json.dump({'stage':'slicer','status':'pass','detail':'stub',"
            "'findings':[]}, open(a.out,'w'))\n"
            "sys.exit(0)\n"
        ) % self.printer_probe
        self._write_stage_stub("slicer", slicer_src)

    def test_slicer_gets_printer_when_sibling_present(self):
        """A printer.json sibling next to the model -> slicer gets --printer."""
        printer_json = os.path.join(self.tmp, "printer.json")
        with open(printer_json, "w", encoding="utf-8") as f:
            json.dump({"nozzle_width": 0.6, "min_perimeters": 3,
                       "layer_height": 0.3, "max_overhang_deg": 50.0}, f)
        self._install_slicer_printer_probe()
        proc = self._run(_env_for())
        self.assertEqual(proc.returncode, 0,
                         "printer-threaded gate must be green; stderr=%s" % proc.stderr)
        got = self._read(self.printer_probe)
        self.assertEqual(got, printer_json,
                         "slicer must receive MODELDIR/printer.json, got %r" % got)

    def test_slicer_no_printer_when_sibling_absent(self):
        """No printer.json sibling -> slicer gets NO --printer (default path)."""
        self._install_slicer_printer_probe()
        proc = self._run(_env_for())
        self.assertEqual(proc.returncode, 0,
                         "absent-printer gate must be green; stderr=%s" % proc.stderr)
        got = self._read(self.printer_probe)
        self.assertEqual(got, "<none>",
                         "slicer must get no --printer when sibling absent, got %r" % got)

    def test_extra_args_for_slicer_printer(self):
        """Direct extra_args_for unit: slicer mirrors load.json->--load wiring."""
        model = os.path.join(self.tmp, "model.json")
        self.assertEqual(extra_args_for("slicer", self.stl, model), [])
        printer_json = os.path.join(self.tmp, "printer.json")
        with open(printer_json, "w", encoding="utf-8") as f:
            f.write("{}")
        self.assertEqual(extra_args_for("slicer", self.stl, model),
                         ["--printer", printer_json])


if __name__ == "__main__":
    unittest.main()

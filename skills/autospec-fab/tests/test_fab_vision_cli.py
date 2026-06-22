"""
test_fab_vision_cli.py — unittest for fab-vision-cli.py.

This is the default $AUTOSPEC_FAB_VISION_CMD consumer. In this iteration the
backend is ALWAYS "none" (no real model), so every outcome is the honest
degradation: empty judge / negative verify, exit 0; usage error exit 2.

The CLI is a hyphenated script (fab-vision-cli.py), so it is exercised exactly
like the real stage exercises it: as a SUBPROCESS with argv + stdin, asserting
on stdout JSON + exit code. (No module import of the hyphenated file.)

Resolver fixtures are MATERIALIZED AT RUNTIME by calling the REAL producer,
stage_render.build_contact_sheet (self-consistent-fixture guard: pin the
resolver against the actual HTML the render stage emits, never a hand-written
copy). PNG bytes are written for the views we want "present" on disk, so the
resolver test exercises real existence-filtering on a real sheet.

Assertions:
  (a) judge `<sheet>`            -> {"observations": []}, exit 0  (backend none)
  (b) verify `--verify <sheet>`  -> {"confirmed": false}, exit 0  (obs on stdin)
  (c) resolver: real build_contact_sheet -> <img src> basenames in document /
      REQUIRED_VIEWS table order; absent PNGs dropped; zero PNGs -> empty.
  (d) missing / unreadable sheet -> {"observations": []}, exit 0.
  (e) usage error (bad flag / no <sheet>) -> exit 2.
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_SCRIPTS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "scripts"))
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

# The REAL contact-sheet producer + the deterministic view order. Importing
# stage_render also makes REQUIRED_VIEWS available (re-exported there).
import stage_render  # noqa: E402

_CLI = os.path.join(_SCRIPTS_DIR, "fab-vision-cli.py")


def _run_cli(args, stdin_text=None):
    """Invoke fab-vision-cli.py as a subprocess; return (rc, parsed_stdout)."""
    proc = subprocess.run(
        [sys.executable, _CLI] + list(args),
        input=stdin_text,
        capture_output=True,
        text=True,
    )
    parsed = None
    out = (proc.stdout or "").strip()
    if out:
        try:
            parsed = json.loads(out)
        except ValueError:
            parsed = None
    return proc.returncode, parsed


def _build_real_sheet(tmpdir, present_view_names):
    """
    Build a real contact sheet via stage_render.build_contact_sheet.

    `present_view_names` is the set of REQUIRED_VIEWS names whose PNG bytes are
    actually written to disk; the rest are referenced by the sheet but absent
    (so the resolver must drop them). Returns (sheet_path, expected_present_pngs)
    where expected_present_pngs is the list of present PNG basenames in the
    deterministic REQUIRED_VIEWS table order.
    """
    render_dir = os.path.join(tmpdir, "renders")
    slug = "widget"
    view_dir = os.path.join(render_dir, slug)
    os.makedirs(view_dir, exist_ok=True)

    results = []
    expected_present = []
    for view in stage_render.REQUIRED_VIEWS:
        name = view["name"]
        png = os.path.join(view_dir, name + ".png")
        present = name in present_view_names
        if present:
            with open(png, "wb") as f:
                f.write(b"\x89PNG\r\n\x1a\n")  # sentinel PNG bytes
            expected_present.append(name + ".png")
        results.append({
            "name": name,
            "kind": view["kind"],
            "png": png,
            "rendered": present,
        })

    sheet = stage_render.build_contact_sheet(render_dir, slug, results)
    return sheet, expected_present


class TestDegradationJudge(unittest.TestCase):

    def test_judge_empty_observations_exit0(self):
        with tempfile.TemporaryDirectory() as tmp:
            sheet, _ = _build_real_sheet(tmp, present_view_names=set())
            rc, out = _run_cli([sheet])
            self.assertEqual(rc, 0)
            self.assertEqual(out, {"observations": []})

    def test_judge_with_rules_arg_empty_exit0(self):
        with tempfile.TemporaryDirectory() as tmp:
            sheet, _ = _build_real_sheet(tmp, present_view_names={"front"})
            rules = os.path.join(tmp, "rules.md")
            with open(rules, "w") as f:
                f.write("# STL Modeling Rules\n")
            rc, out = _run_cli([sheet, rules])
            self.assertEqual(rc, 0)
            self.assertEqual(out, {"observations": []})


class TestDegradationVerify(unittest.TestCase):

    def test_verify_confirmed_false_exit0(self):
        with tempfile.TemporaryDirectory() as tmp:
            sheet, _ = _build_real_sheet(tmp, present_view_names={"front"})
            candidate = json.dumps({
                "observation": "left gasket groove appears exposed",
                "severity": "warn",
                "view": "left",
                "rule": "exposed_gasket",
            })
            rc, out = _run_cli(["--verify", sheet], stdin_text=candidate)
            self.assertEqual(rc, 0)
            self.assertEqual(out, {"confirmed": False})

    def test_verify_empty_stdin_confirmed_false_exit0(self):
        with tempfile.TemporaryDirectory() as tmp:
            sheet, _ = _build_real_sheet(tmp, present_view_names=set())
            rc, out = _run_cli(["--verify", sheet], stdin_text="")
            self.assertEqual(rc, 0)
            self.assertEqual(out, {"confirmed": False})


class TestMissingSheet(unittest.TestCase):

    def test_missing_sheet_empty_judge_exit0(self):
        with tempfile.TemporaryDirectory() as tmp:
            missing = os.path.join(tmp, "nope", "contact-sheet.html")
            rc, out = _run_cli([missing])
            self.assertEqual(rc, 0)
            self.assertEqual(out, {"observations": []})


class TestUsageError(unittest.TestCase):

    def test_no_sheet_positional_exit2(self):
        rc, _ = _run_cli([])
        self.assertEqual(rc, 2)

    def test_bad_flag_exit2(self):
        rc, _ = _run_cli(["--definitely-not-a-flag", "sheet.html"])
        self.assertEqual(rc, 2)


class TestResolver(unittest.TestCase):
    """Resolver pinned against the REAL build_contact_sheet output."""

    def setUp(self):
        # Import the resolver here so a hyphenated-module import failure surfaces
        # as a clear skip rather than a collection error. The CLI file name has a
        # hyphen, so we load it via importlib from its path.
        import importlib.util
        spec = importlib.util.spec_from_file_location("fab_vision_cli", _CLI)
        self.mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.mod)

    def test_all_present_pngs_in_table_order(self):
        with tempfile.TemporaryDirectory() as tmp:
            # Present every required view. REQUIRED_VIEWS has 17 entries, above
            # the default image cap (16), so lift the cap for this assertion so
            # we exercise full table-order resolution rather than the cap.
            os.environ["AUTOSPEC_FAB_VISION_MAX_IMAGES"] = "100"
            self.addCleanup(os.environ.pop, "AUTOSPEC_FAB_VISION_MAX_IMAGES",
                            None)
            all_names = {v["name"] for v in stage_render.REQUIRED_VIEWS}
            sheet, expected = _build_real_sheet(tmp, present_view_names=all_names)
            resolved = self.mod.resolve_images(sheet)
            got = [os.path.basename(p) for p in resolved]
            self.assertEqual(got, expected)
            # Document/table order == REQUIRED_VIEWS order.
            self.assertEqual(
                got, [v["name"] + ".png" for v in stage_render.REQUIRED_VIEWS])

    def test_absent_pngs_dropped(self):
        with tempfile.TemporaryDirectory() as tmp:
            present = {"front", "left", "top", "iso-front-right-top"}
            sheet, expected = _build_real_sheet(tmp, present_view_names=present)
            resolved = self.mod.resolve_images(sheet)
            got = [os.path.basename(p) for p in resolved]
            # Only present PNGs survive, in deterministic table order.
            self.assertEqual(got, expected)
            self.assertEqual(len(got), len(present))

    def test_zero_pngs_empty(self):
        with tempfile.TemporaryDirectory() as tmp:
            sheet, expected = _build_real_sheet(tmp, present_view_names=set())
            self.assertEqual(expected, [])
            self.assertEqual(self.mod.resolve_images(sheet), [])

    def test_missing_sheet_empty(self):
        with tempfile.TemporaryDirectory() as tmp:
            missing = os.path.join(tmp, "nope.html")
            self.assertEqual(self.mod.resolve_images(missing), [])

    def test_cap_trims_to_max_images_in_order(self):
        with tempfile.TemporaryDirectory() as tmp:
            os.environ["AUTOSPEC_FAB_VISION_MAX_IMAGES"] = "3"
            self.addCleanup(os.environ.pop, "AUTOSPEC_FAB_VISION_MAX_IMAGES",
                            None)
            all_names = {v["name"] for v in stage_render.REQUIRED_VIEWS}
            sheet, expected = _build_real_sheet(tmp, present_view_names=all_names)
            got = [os.path.basename(p) for p in self.mod.resolve_images(sheet)]
            self.assertEqual(got, expected[:3])


if __name__ == "__main__":
    unittest.main()

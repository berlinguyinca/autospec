"""test_stage_fea.py — unittest for stage_fea.py (CalculiX FEA stage).

Covers pass/fail/skip verdicts, the geometry-hash cache, ccx-absent skip, the
stage-record fragment shape, and the structured fea_results object (emitted on
pass/fail; absent on skip). The ccx shim writes an invocation counter to
$CCX_SHIM_COUNT_FILE and a synthetic .dat whose safety factor is $CCX_SHIM_SAFETY.
"""

import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_SCRIPTS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "scripts"))
_FIXTURES_DIR = os.path.normpath(os.path.join(_THIS_DIR, "fixtures", "fea"))

_STAGE_SCRIPT = os.path.join(_SCRIPTS_DIR, "stage_fea.py")

_CRITICAL_MODEL = os.path.join(_FIXTURES_DIR, "model-critical.json")
_NONCRITICAL_MODEL = os.path.join(_FIXTURES_DIR, "model-noncritical.json")
_BODY_STL = os.path.join(_FIXTURES_DIR, "body.stl")
_LOAD_JSON = os.path.join(_FIXTURES_DIR, "load.json")

# Safety factor above/below the default minimum (2.0)
_HIGH_SAFETY = "3.5"
_LOW_SAFETY = "0.8"


def _make_ccx_shim(bin_dir: str, safety_factor: str, count_file: str) -> str:
    """Write a ccx shim to bin_dir/ccx.

    The shim increments the counter in count_file, writes a <job>.dat with a
    SAFETY_FACTOR line, and exits 0. Env-driven: CCX_SHIM_SAFETY (default 3.5),
    CCX_SHIM_COUNT_FILE (counter path).
    """
    shim_path = os.path.join(bin_dir, "ccx")
    shim_content = f"""\
#!/bin/sh
# ccx shim for autospec-fab FEA tests
count_file="${{CCX_SHIM_COUNT_FILE:-{count_file}}}"
if [ -f "$count_file" ]; then
    count=$(cat "$count_file")
else
    count=0
fi
echo $((count + 1)) > "$count_file"

safety="${{CCX_SHIM_SAFETY:-{safety_factor}}}"
# The last positional arg to ccx is the job name (no extension).
job_name=""
for arg do
    job_name="$arg"
done
dat_file="${{job_name}}.dat"

cat > "$dat_file" << EOF2
 S T E P   1
 MAX_MISES ${{safety}}e+08
 SAFETY_FACTOR ${{safety}}
EOF2

exit 0
"""
    with open(shim_path, "w") as fh:
        fh.write(shim_content)
    os.chmod(shim_path, stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP | stat.S_IROTH | stat.S_IXOTH)
    return shim_path


def _run_stage(
    stl_path: str,
    model_path: str,
    extra_env: dict | None = None,
    cache_dir: str | None = None,
) -> tuple[int, dict | None, str]:
    """Run stage_fea.py; return (returncode, fragment_dict, stderr).

    extra_env merges over os.environ; cache_dir sets AUTOSPEC_FAB_CACHE_DIR.
    """
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
        out_path = tf.name
    try:
        env = os.environ.copy()
        if extra_env:
            env.update(extra_env)
        if cache_dir:
            env["AUTOSPEC_FAB_CACHE_DIR"] = cache_dir

        cmd = [
            sys.executable, _STAGE_SCRIPT,
            "--in", stl_path,
            "--model", model_path,
            "--out", out_path,
            "--load", _LOAD_JSON,
        ]
        result = subprocess.run(cmd, capture_output=True, text=True, env=env)
        fragment = None
        if os.path.exists(out_path):
            with open(out_path) as f:
                fragment = json.load(f)
        return result.returncode, fragment, result.stderr
    finally:
        if os.path.exists(out_path):
            os.unlink(out_path)


def _finding_codes(fragment: dict) -> set:
    return {f.get("code") for f in fragment.get("findings", [])}


def _read_count(count_file: str) -> int:
    if not os.path.exists(count_file):
        return 0
    with open(count_file) as f:
        return int(f.read().strip())


class _WithCcxShim(unittest.TestCase):
    """Base class: install a $TMP/bin/ccx shim before each test."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="fea_test_")
        self.bin_dir = os.path.join(self.tmp, "bin")
        os.makedirs(self.bin_dir)
        self.count_file = os.path.join(self.tmp, "ccx_count.txt")
        self.cache_dir = os.path.join(self.tmp, "fea-cache")
        os.makedirs(self.cache_dir)
        self.env_base = {
            "PATH": self.bin_dir + ":" + os.environ.get("PATH", ""),
            "CCX_SHIM_COUNT_FILE": self.count_file,
        }

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _install_shim(self, safety: str):
        _make_ccx_shim(self.bin_dir, safety, self.count_file)

    def _run(self, safety: str, model: str | None = None) -> tuple[int, dict | None, str]:
        env = dict(self.env_base)
        env["CCX_SHIM_SAFETY"] = safety
        return _run_stage(
            _BODY_STL,
            model or _CRITICAL_MODEL,
            extra_env=env,
            cache_dir=self.cache_dir,
        )


class TestFragmentShape(_WithCcxShim):
    def _assert_valid_fragment(self, fragment, label):
        self.assertIsInstance(fragment, dict, f"{label}: fragment must be a dict")
        for key in ("stage", "status", "detail", "findings"):
            self.assertIn(key, fragment, f"{label}: fragment missing '{key}'")
        self.assertIn(
            fragment["status"], {"pass", "fail", "warn", "skip"},
            f"{label}: invalid status {fragment['status']!r}",
        )
        self.assertIsInstance(fragment["findings"], list,
                              f"{label}: 'findings' must be a list")
        self.assertEqual(fragment["stage"], "fea",
                         f"{label}: stage name must be 'fea'")

    def test_strong_part_fragment_shape(self):
        self._install_shim(_HIGH_SAFETY)
        _, fragment, _ = self._run(_HIGH_SAFETY)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment, "strong_part")

    def test_weak_part_fragment_shape(self):
        self._install_shim(_LOW_SAFETY)
        _, fragment, _ = self._run(_LOW_SAFETY)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment, "weak_part")

    def test_noncritical_fragment_shape(self):
        self._install_shim(_HIGH_SAFETY)
        env = dict(self.env_base)
        env["CCX_SHIM_SAFETY"] = _HIGH_SAFETY
        _, fragment, _ = _run_stage(
            _BODY_STL, _NONCRITICAL_MODEL,
            extra_env=env,
            cache_dir=self.cache_dir,
        )
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment, "noncritical")


class TestStrongPart(_WithCcxShim):
    def setUp(self):
        super().setUp()
        self._install_shim(_HIGH_SAFETY)
        self.rc, self.fragment, self.stderr = self._run(_HIGH_SAFETY)

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0,
                         f"Expected harness exit 0; stderr={self.stderr}")

    def test_status_pass(self):
        self.assertEqual(
            self.fragment["status"], "pass",
            f"Expected pass for strong part; fragment={self.fragment}",
        )

    def test_no_fea_below_safety_finding(self):
        codes = _finding_codes(self.fragment)
        self.assertNotIn("fea_below_safety", codes,
                         f"Unexpected fea_below_safety; findings={self.fragment['findings']}")

    def test_ccx_was_invoked(self):
        count = _read_count(self.count_file)
        self.assertGreater(count, 0, "Expected ccx shim to be invoked at least once")

    def test_fea_results_carries_safety_factor(self):
        results = self.fragment.get("fea_results")
        self.assertIsInstance(
            results, dict,
            f"Expected structured fea_results dict; fragment={self.fragment}",
        )
        self.assertAlmostEqual(results["safety_factor"], float(_HIGH_SAFETY))
        self.assertEqual(results["required_min"], 2.0)
        self.assertEqual(results["status"], "pass")


class TestWeakPart(_WithCcxShim):
    """A load-critical part with low safety factor must fail + emit fea_below_safety."""

    def setUp(self):
        super().setUp()
        self._install_shim(_LOW_SAFETY)
        self.rc, self.fragment, self.stderr = self._run(_LOW_SAFETY)

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0,
                         f"Expected harness exit 0; stderr={self.stderr}")

    def test_status_fail(self):
        self.assertEqual(
            self.fragment["status"], "fail",
            f"Expected fail for weak part; fragment={self.fragment}",
        )

    def test_finding_fea_below_safety(self):
        codes = _finding_codes(self.fragment)
        self.assertIn("fea_below_safety", codes,
                      f"Expected fea_below_safety finding; findings={self.fragment['findings']}")

    def test_fea_results_carries_fail_status(self):
        results = self.fragment.get("fea_results")
        self.assertIsInstance(results, dict,
                              f"Expected fea_results dict; fragment={self.fragment}")
        self.assertAlmostEqual(results["safety_factor"], float(_LOW_SAFETY))
        self.assertEqual(results["required_min"], 2.0)
        self.assertEqual(results["status"], "fail")


class TestNonCriticalPart(_WithCcxShim):
    """A part with load_critical: false must skip without running ccx."""

    def setUp(self):
        super().setUp()
        self._install_shim(_HIGH_SAFETY)
        env = dict(self.env_base)
        env["CCX_SHIM_SAFETY"] = _HIGH_SAFETY
        self.rc, self.fragment, self.stderr = _run_stage(
            _BODY_STL, _NONCRITICAL_MODEL,
            extra_env=env,
            cache_dir=self.cache_dir,
        )

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0,
                         f"Expected harness exit 0; stderr={self.stderr}")

    def test_status_skip(self):
        self.assertEqual(
            self.fragment["status"], "skip",
            f"Expected skip for non-critical part; fragment={self.fragment}",
        )

    def test_detail_mentions_not_load_critical(self):
        self.assertIn(
            "not load-critical", self.fragment["detail"],
            f"Expected 'not load-critical' in detail; detail={self.fragment['detail']!r}",
        )

    def test_ccx_not_invoked(self):
        count = _read_count(self.count_file)
        self.assertEqual(count, 0, "ccx must not run for non-load-critical parts")

    def test_no_fabricated_fea_results_on_skip(self):
        self.assertNotIn(
            "fea_results", self.fragment,
            "skip path must not fabricate fea_results numbers",
        )


class TestCacheHit(_WithCcxShim):
    """Re-running the same inputs must hit the cache; ccx must not be re-invoked."""

    def test_second_run_hits_cache(self):
        self._install_shim(_HIGH_SAFETY)

        # First run — should invoke ccx once
        rc1, frag1, _ = self._run(_HIGH_SAFETY)
        self.assertEqual(rc1, 0)
        self.assertEqual(frag1["status"], "pass")
        count_after_first = _read_count(self.count_file)
        self.assertGreater(count_after_first, 0, "ccx must be called on first run")

        # Reset counter so the second call's count is unambiguous.
        with open(self.count_file, "w") as f:
            f.write("0")

        # Second run — identical inputs, same cache dir → should hit cache
        rc2, frag2, _ = self._run(_HIGH_SAFETY)
        self.assertEqual(rc2, 0)
        self.assertEqual(frag2["status"], "pass",
                         f"Cached result must still be pass; fragment={frag2}")

        count_after_second = _read_count(self.count_file)
        self.assertEqual(
            count_after_second, 0,
            f"ccx must NOT be invoked on cache hit (count={count_after_second})",
        )

    def test_changed_orientation_busts_cache(self):
        """A changed print_orientation must MISS the cache against the SAME cache
        dir — proving the geometry-hash key includes print_orientation."""
        self._install_shim(_HIGH_SAFETY)

        # Prime the cache with the upright critical model.
        rc1, _, _ = self._run(_HIGH_SAFETY, model=_CRITICAL_MODEL)
        self.assertEqual(rc1, 0)
        self.assertGreater(_read_count(self.count_file), 0)
        with open(self.count_file, "w") as f:
            f.write("0")

        # Build a variant that differs ONLY in print_orientation.
        with open(_CRITICAL_MODEL) as f:
            variant = json.load(f)
        self.assertEqual(variant["print_orientation"], "upright")
        variant["print_orientation"] = "flat"
        variant_path = os.path.join(self.tmp, "model-critical-flat.json")
        with open(variant_path, "w") as f:
            json.dump(variant, f)

        # Same cache dir → different hash → cache MISS → ccx re-runs.
        rc2, _, _ = self._run(_HIGH_SAFETY, model=variant_path)
        self.assertEqual(rc2, 0)
        self.assertGreater(
            _read_count(self.count_file), 0,
            "ccx MUST re-run when print_orientation changes (hash key must "
            "include print_orientation)",
        )


class TestCcxAbsent(unittest.TestCase):
    """When ccx is not on PATH, stage must skip (solver deferred to container)."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="fea_nobin_")
        self.cache_dir = os.path.join(self.tmp, "fea-cache")
        os.makedirs(self.cache_dir)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_skip_when_no_ccx(self):
        # Use an empty bin dir so ccx is not resolvable
        empty_bin = os.path.join(self.tmp, "emptybin")
        os.makedirs(empty_bin)
        env = os.environ.copy()
        env["PATH"] = empty_bin + ":" + self.tmp
        env["AUTOSPEC_FAB_CACHE_DIR"] = self.cache_dir

        rc, fragment, stderr = _run_stage(
            _BODY_STL, _CRITICAL_MODEL,
            extra_env=env,
            cache_dir=self.cache_dir,
        )
        self.assertEqual(rc, 0, f"Harness must exit 0; stderr={stderr}")
        self.assertEqual(
            fragment["status"], "skip",
            f"Expected skip when ccx absent; fragment={fragment}",
        )
        self.assertEqual(fragment["findings"], [],
                         "No findings when ccx absent (solver deferred)")
        self.assertNotIn(
            "fea_results", fragment,
            "ccx-absent skip must not fabricate fea_results",
        )


if __name__ == "__main__":
    unittest.main()

"""
test_stage_cfd.py — unittest for stage_cfd.py (OpenFOAM CFD stage).

Tests:
  1. passing_part:   shim emits targets-met metrics       → status pass.
  2. target_miss:    shim emits over-limit pressure drop  → status fail + cfd_target_miss.
  3. non_critical model (flow_critical: false)            → status skip.
  4. cache hit: re-run same inputs                        → solver invoked 0 times second run.
  5. cache bust: changed print_orientation, SAME cache dir → solver re-invoked (hash miss).
  6. solver absent from PATH                             → status skip (solver deferred).
  7. fragment validates release-gate stage-record shape.

The simpleFoam shim writes an invocation counter to $SHIM_COUNT_FILE and emits
a synthetic ``cfd_results`` file whose metrics are controlled by env vars:
  SHIM_PRESSURE_DROP  — pressure_drop_pa to emit  (default 200.0)
  SHIM_MIN_VELOCITY   — min_velocity_m_s to emit  (default 3.0)
  SHIM_STAGNATION     — 0 or 1 stagnation flag    (default 0)
  SHIM_COUNT_FILE     — path to invocation counter file
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
_FIXTURES_DIR = os.path.normpath(os.path.join(_THIS_DIR, "fixtures", "cfd"))

_STAGE_SCRIPT = os.path.join(_SCRIPTS_DIR, "stage_cfd.py")

_CRITICAL_MODEL = os.path.join(_FIXTURES_DIR, "model-critical.json")
_NONCRITICAL_MODEL = os.path.join(_FIXTURES_DIR, "model-noncritical.json")
_BODY_STL = os.path.join(_FIXTURES_DIR, "body.stl")
_FLOW_JSON = os.path.join(_FIXTURES_DIR, "flow.json")

# Metrics above/below the flow.json targets
# flow.json: max_pressure_drop_pa=500, min_velocity_m_s=1.0
_PASSING_PRESSURE = "200.0"   # < 500 Pa  → pass
_PASSING_VELOCITY = "3.0"     # > 1.0 m/s → pass
_FAILING_PRESSURE = "800.0"   # > 500 Pa  → target miss
_FAILING_VELOCITY = "0.5"     # < 1.0 m/s → target miss


def _make_solver_shim(bin_dir: str, count_file: str) -> str:
    """
    Write a simpleFoam shim to bin_dir/simpleFoam.

    The shim:
      - Increments the integer in count_file by 1 (creates it at 0 first).
      - Writes a ``cfd_results`` file in the current directory (the case dir)
        with metric values taken from env vars at invocation time.
      - Exits 0.

    Env vars (read at shim-invocation time):
      SHIM_PRESSURE_DROP  — float pressure_drop_pa (default 200.0)
      SHIM_MIN_VELOCITY   — float min_velocity_m_s (default 3.0)
      SHIM_STAGNATION     — int 0|1               (default 0)
      SHIM_COUNT_FILE     — path to counter file
    """
    shim_path = os.path.join(bin_dir, "simpleFoam")
    shim_content = f"""\
#!/bin/sh
# simpleFoam shim for autospec-fab CFD tests

# --- increment invocation counter ---
count_file="${{SHIM_COUNT_FILE:-{count_file}}}"
if [ -f "$count_file" ]; then
    count=$(cat "$count_file")
else
    count=0
fi
echo $((count + 1)) > "$count_file"

# --- emit synthetic cfd_results ---
pressure="${{SHIM_PRESSURE_DROP:-200.0}}"
velocity="${{SHIM_MIN_VELOCITY:-3.0}}"
stagnation="${{SHIM_STAGNATION:-0}}"

cat > cfd_results << EOF2
PRESSURE_DROP ${{pressure}}
MIN_VELOCITY ${{velocity}}
STAGNATION ${{stagnation}}
EOF2

exit 0
"""
    with open(shim_path, "w") as fh:
        fh.write(shim_content)
    os.chmod(
        shim_path,
        stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP | stat.S_IROTH | stat.S_IXOTH,
    )
    return shim_path


def _run_stage(
    stl_path: str,
    model_path: str,
    extra_env: dict | None = None,
    cache_dir: str | None = None,
) -> tuple[int, dict | None, str]:
    """
    Run stage_cfd.py; return (returncode, fragment_dict, stderr).

    extra_env — env vars merged on top of the current environment.
    cache_dir — if given, set AUTOSPEC_FAB_CACHE_DIR for an isolated cache.
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
            "--flow", _FLOW_JSON,
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


class _WithSolverShim(unittest.TestCase):
    """Base class: install a $TMP/bin/simpleFoam shim before each test."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="cfd_test_")
        self.bin_dir = os.path.join(self.tmp, "bin")
        os.makedirs(self.bin_dir)
        self.count_file = os.path.join(self.tmp, "shim_count.txt")
        self.cache_dir = os.path.join(self.tmp, "cfd-cache")
        os.makedirs(self.cache_dir)
        self.env_base = {
            "PATH": self.bin_dir + ":" + os.environ.get("PATH", ""),
            "SHIM_COUNT_FILE": self.count_file,
        }
        _make_solver_shim(self.bin_dir, self.count_file)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _run(
        self,
        pressure: str = _PASSING_PRESSURE,
        velocity: str = _PASSING_VELOCITY,
        stagnation: str = "0",
        model: str | None = None,
    ) -> tuple[int, dict | None, str]:
        env = dict(self.env_base)
        env["SHIM_PRESSURE_DROP"] = pressure
        env["SHIM_MIN_VELOCITY"] = velocity
        env["SHIM_STAGNATION"] = stagnation
        return _run_stage(
            _BODY_STL,
            model or _CRITICAL_MODEL,
            extra_env=env,
            cache_dir=self.cache_dir,
        )


class TestFragmentShape(_WithSolverShim):
    """Every fragment must match release-gate stage-record shape."""

    def _assert_valid_fragment(self, fragment, label):
        self.assertIsInstance(fragment, dict, f"{label}: fragment must be a dict")
        for key in ("stage", "status", "detail", "findings"):
            self.assertIn(key, fragment, f"{label}: fragment missing '{key}'")
        self.assertIn(
            fragment["status"], {"pass", "fail", "warn", "skip"},
            f"{label}: invalid status {fragment['status']!r}",
        )
        self.assertIsInstance(
            fragment["findings"], list, f"{label}: 'findings' must be a list"
        )
        self.assertEqual(
            fragment["stage"], "cfd", f"{label}: stage name must be 'cfd'"
        )

    def test_passing_part_fragment_shape(self):
        _, fragment, _ = self._run()
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment, "passing_part")

    def test_target_miss_fragment_shape(self):
        _, fragment, _ = self._run(pressure=_FAILING_PRESSURE)
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment, "target_miss")

    def test_noncritical_fragment_shape(self):
        env = dict(self.env_base)
        _, fragment, _ = _run_stage(
            _BODY_STL, _NONCRITICAL_MODEL,
            extra_env=env,
            cache_dir=self.cache_dir,
        )
        self.assertIsNotNone(fragment)
        self._assert_valid_fragment(fragment, "noncritical")


class TestPassingPart(_WithSolverShim):
    """A flow-critical part meeting all targets must pass."""

    def setUp(self):
        super().setUp()
        self.rc, self.fragment, self.stderr = self._run()

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0, f"Expected harness exit 0; stderr={self.stderr}")

    def test_status_pass(self):
        self.assertEqual(
            self.fragment["status"], "pass",
            f"Expected pass for passing part; fragment={self.fragment}",
        )

    def test_no_cfd_target_miss_finding(self):
        codes = _finding_codes(self.fragment)
        self.assertNotIn(
            "cfd_target_miss", codes,
            f"Unexpected cfd_target_miss; findings={self.fragment['findings']}",
        )

    def test_solver_was_invoked(self):
        count = _read_count(self.count_file)
        self.assertGreater(count, 0, "Expected solver shim to be invoked at least once")


class TestTargetMiss(_WithSolverShim):
    """A flow-critical part exceeding pressure-drop target must fail + emit cfd_target_miss."""

    def setUp(self):
        super().setUp()
        self.rc, self.fragment, self.stderr = self._run(pressure=_FAILING_PRESSURE)

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0, f"Expected harness exit 0; stderr={self.stderr}")

    def test_status_fail(self):
        self.assertEqual(
            self.fragment["status"], "fail",
            f"Expected fail for target miss; fragment={self.fragment}",
        )

    def test_finding_cfd_target_miss(self):
        codes = _finding_codes(self.fragment)
        self.assertIn(
            "cfd_target_miss", codes,
            f"Expected cfd_target_miss finding; findings={self.fragment['findings']}",
        )


class TestVelocityMiss(_WithSolverShim):
    """A flow-critical part with min_velocity below target must fail."""

    def test_low_velocity_fails(self):
        rc, fragment, stderr = self._run(velocity=_FAILING_VELOCITY)
        self.assertEqual(rc, 0, f"Harness exit 0 expected; stderr={stderr}")
        self.assertEqual(
            fragment["status"], "fail",
            f"Expected fail for low velocity; fragment={fragment}",
        )
        self.assertIn("cfd_target_miss", _finding_codes(fragment))


class TestStagnationMiss(_WithSolverShim):
    """A part reporting stagnation when no_stagnation=true must fail."""

    def test_stagnation_fails(self):
        rc, fragment, stderr = self._run(stagnation="1")
        self.assertEqual(rc, 0, f"Harness exit 0 expected; stderr={stderr}")
        self.assertEqual(
            fragment["status"], "fail",
            f"Expected fail when stagnation detected; fragment={fragment}",
        )
        self.assertIn("cfd_target_miss", _finding_codes(fragment))


class TestNonCriticalPart(_WithSolverShim):
    """A part with flow_critical: false must skip without running the solver."""

    def setUp(self):
        super().setUp()
        env = dict(self.env_base)
        self.rc, self.fragment, self.stderr = _run_stage(
            _BODY_STL, _NONCRITICAL_MODEL,
            extra_env=env,
            cache_dir=self.cache_dir,
        )

    def test_exit_code_zero(self):
        self.assertEqual(self.rc, 0, f"Expected harness exit 0; stderr={self.stderr}")

    def test_status_skip(self):
        self.assertEqual(
            self.fragment["status"], "skip",
            f"Expected skip for non-critical part; fragment={self.fragment}",
        )

    def test_detail_mentions_not_flow_critical(self):
        self.assertIn(
            "not flow-critical", self.fragment["detail"],
            f"Expected 'not flow-critical' in detail; detail={self.fragment['detail']!r}",
        )

    def test_solver_not_invoked(self):
        count = _read_count(self.count_file)
        self.assertEqual(count, 0, "Solver must not run for non-flow-critical parts")


class TestCacheHit(_WithSolverShim):
    """Re-running the same inputs must hit the cache; solver must not be re-invoked."""

    def test_second_run_hits_cache(self):
        # First run — should invoke solver once
        rc1, frag1, _ = self._run()
        self.assertEqual(rc1, 0)
        self.assertEqual(frag1["status"], "pass")
        count_after_first = _read_count(self.count_file)
        self.assertGreater(count_after_first, 0, "Solver must be called on first run")

        # Reset counter so the second call's count is unambiguous
        with open(self.count_file, "w") as f:
            f.write("0")

        # Second run — identical inputs, same cache dir → cache HIT
        rc2, frag2, _ = self._run()
        self.assertEqual(rc2, 0)
        self.assertEqual(
            frag2["status"], "pass",
            f"Cached result must still be pass; fragment={frag2}",
        )

        count_after_second = _read_count(self.count_file)
        self.assertEqual(
            count_after_second, 0,
            f"Solver must NOT be invoked on cache hit (count={count_after_second})",
        )

    def test_changed_orientation_busts_cache(self):
        """A changed print_orientation must MISS the cache against the SAME cache
        dir — proving the geometry-hash key includes print_orientation.

        Strategy: prime the cache with the upright critical model, then re-run
        against the SAME cache dir with a variant differing ONLY in
        print_orientation. The hash must differ → cold miss → solver re-invoked.
        Using a fresh cache dir would only prove cold-cache behaviour and cannot
        catch a regression that dropped orientation from the hash key.
        """
        # Prime the cache with the original (upright) model
        rc1, _, _ = self._run(model=_CRITICAL_MODEL)
        self.assertEqual(rc1, 0)
        self.assertGreater(_read_count(self.count_file), 0)

        # Reset invocation counter so the second run's count is unambiguous
        with open(self.count_file, "w") as f:
            f.write("0")

        # Build a variant differing ONLY in print_orientation
        with open(_CRITICAL_MODEL) as f:
            variant = json.load(f)
        self.assertEqual(variant["print_orientation"], "upright")
        variant["print_orientation"] = "flat"
        variant_path = os.path.join(self.tmp, "model-critical-flat.json")
        with open(variant_path, "w") as f:
            json.dump(variant, f)

        # Re-run against the SAME cache dir → different hash → cache MISS → solver runs
        rc2, _, _ = self._run(model=variant_path)
        self.assertEqual(rc2, 0)
        self.assertGreater(
            _read_count(self.count_file), 0,
            "Solver MUST be re-invoked when print_orientation changes against the "
            "same cache dir (hash key must include print_orientation)",
        )

    def test_changed_flow_spec_busts_cache(self):
        """A changed flow target (max_pressure_drop_pa) must also bust the cache
        against the SAME cache dir — proving the flow spec feeds the hash."""
        # Prime cache
        rc1, _, _ = self._run()
        self.assertEqual(rc1, 0)
        self.assertGreater(_read_count(self.count_file), 0)

        # Reset counter
        with open(self.count_file, "w") as f:
            f.write("0")

        # Write a variant flow.json with a different pressure target
        with open(_FLOW_JSON) as f:
            variant_flow = json.load(f)
        variant_flow["max_pressure_drop_pa"] = 999.0
        variant_flow_path = os.path.join(self.tmp, "flow-variant.json")
        with open(variant_flow_path, "w") as f:
            json.dump(variant_flow, f)

        # Run with variant flow against the SAME cache dir
        env = dict(self.env_base)
        env["SHIM_PRESSURE_DROP"] = _PASSING_PRESSURE
        env["SHIM_MIN_VELOCITY"] = _PASSING_VELOCITY
        env["SHIM_STAGNATION"] = "0"
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        try:
            cmd = [
                sys.executable, _STAGE_SCRIPT,
                "--in", _BODY_STL,
                "--model", _CRITICAL_MODEL,
                "--out", out_path,
                "--flow", variant_flow_path,
            ]
            env2 = os.environ.copy()
            env2.update(env)
            env2["AUTOSPEC_FAB_CACHE_DIR"] = self.cache_dir
            subprocess.run(cmd, capture_output=True, text=True, env=env2)
        finally:
            if os.path.exists(out_path):
                os.unlink(out_path)

        self.assertGreater(
            _read_count(self.count_file), 0,
            "Solver MUST be re-invoked when flow spec changes (hash key must include flow)",
        )


class TestSolverAbsent(unittest.TestCase):
    """When simpleFoam is not on PATH, stage must skip (solver deferred to container)."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="cfd_nobin_")
        self.cache_dir = os.path.join(self.tmp, "cfd-cache")
        os.makedirs(self.cache_dir)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_skip_when_no_solver(self):
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
            f"Expected skip when solver absent; fragment={fragment}",
        )
        self.assertEqual(fragment["findings"], [],
                         "No findings when solver absent (deferred to container)")


if __name__ == "__main__":
    unittest.main()

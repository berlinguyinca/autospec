"""
test_stage_cfd_cache.py — cache-behaviour unittests for stage_cfd.py.

Split out of test_stage_cfd.py to keep each test module under the
AUTOSPEC_MAX_FILE_LOC limit. Reuses the solver-shim base class and helpers
from test_stage_cfd.

Tests:
  - cache hit: re-run same inputs           → solver invoked 0 times second run.
  - cache bust (orientation): changed print_orientation, SAME cache dir → re-run.
  - cache bust (flow spec):   changed max_pressure_drop_pa            → re-run.
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

from test_stage_cfd import (
    _BODY_STL,
    _CRITICAL_MODEL,
    _FLOW_JSON,
    _PASSING_PRESSURE,
    _PASSING_VELOCITY,
    _STAGE_SCRIPT,
    _WithSolverShim,
    _read_count,
)


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


if __name__ == "__main__":
    unittest.main()

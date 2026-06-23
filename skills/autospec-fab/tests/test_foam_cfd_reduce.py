"""test_foam_cfd_reduce.py — unit tests for the OpenFOAM→cfd_results reducer.

The reducer bridges real OpenFOAM time-dir field output (p/U) to stage_cfd.py's
``cfd_results`` parse contract. The load-bearing assertion: the cfd_results file the
reducer writes is parsed by stage_cfd.py's *real* ``_parse_cfd_results`` (imported
here, not re-implemented) back to the values we reduced. Sample OpenFOAM field files
are materialized at runtime so the test runs on a clean checkout.
"""

import math
import os
import sys
import tempfile
import unittest

_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_SCRIPTS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "scripts"))
_WRAPPERS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "docker", "wrappers"))

sys.path.insert(0, _SCRIPTS_DIR)
sys.path.insert(0, _WRAPPERS_DIR)

import foam_cfd_reduce as reduce_mod  # noqa: E402
import stage_cfd  # noqa: E402  (the real, unchanged stage parser is the oracle)


_P_NONUNIFORM = """\
FoamFile { version 2.0; format ascii; class volScalarField; object p; }
dimensions      [0 2 -2 0 0 0 0];
internalField   nonuniform List<scalar>
4
(
100.0
50.0
0.0
20.0
)
;
boundaryField { outlet { type zeroGradient; } }
"""

# Velocities: |U| = 5, 5, 0.01 (near-stagnation), 3 → min = 0.01.
_U_NONUNIFORM = """\
FoamFile { version 2.0; format ascii; class volVectorField; object U; }
dimensions      [0 1 -1 0 0 0 0];
internalField   nonuniform List<vector>
4
(
(5 0 0)
(0 5 0)
(0.01 0 0)
(3 0 0)
)
;
boundaryField { inlet { type fixedValue; } }
"""

_P_UNIFORM = "internalField   uniform 42.0;\n"
_U_UNIFORM = "internalField   uniform (2 0 0);\n"


class TestFieldParsers(unittest.TestCase):
    def test_scalar_nonuniform(self):
        vals = reduce_mod.parse_scalar_field(_P_NONUNIFORM)
        self.assertEqual(sorted(vals), [0.0, 20.0, 50.0, 100.0])

    def test_scalar_uniform(self):
        self.assertEqual(reduce_mod.parse_scalar_field(_P_UNIFORM), [42.0])

    def test_vector_nonuniform(self):
        vecs = reduce_mod.parse_vector_field(_U_NONUNIFORM)
        self.assertEqual(len(vecs), 4)
        self.assertIn((0.01, 0.0, 0.0), vecs)

    def test_vector_uniform(self):
        self.assertEqual(reduce_mod.parse_vector_field(_U_UNIFORM), [(2.0, 0.0, 0.0)])


class TestReductions(unittest.TestCase):
    def test_pressure_drop(self):
        # (100 - 0) * rho. rho=1.0 → 100 Pa.
        dp = reduce_mod.reduce_pressure_drop([100.0, 50.0, 0.0, 20.0], 1.0)
        self.assertAlmostEqual(dp, 100.0, places=6)

    def test_min_velocity(self):
        vel = reduce_mod.reduce_min_velocity([(5, 0, 0), (0.01, 0, 0), (3, 0, 0)])
        self.assertAlmostEqual(vel, 0.01, places=6)

    def test_empty_returns_none(self):
        self.assertIsNone(reduce_mod.reduce_pressure_drop([], 1.0))
        self.assertIsNone(reduce_mod.reduce_min_velocity([]))


class TestContractRoundTrip(unittest.TestCase):
    """The crux: reducer output must be parsed by stage_cfd.py's real parser."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="foamred_")
        # Latest time dir "100" beats "0"; reducer must pick 100.
        self.t100 = os.path.join(self.tmp, "100")
        self.t0 = os.path.join(self.tmp, "0")
        os.makedirs(self.t100)
        os.makedirs(self.t0)
        # Decoy values in the 0 dir to prove latest-time selection.
        with open(os.path.join(self.t0, "p"), "w") as fh:
            fh.write("internalField   uniform 999.0;\n")
        with open(os.path.join(self.t100, "p"), "w") as fh:
            fh.write(_P_NONUNIFORM)
        with open(os.path.join(self.t100, "U"), "w") as fh:
            fh.write(_U_NONUNIFORM)

    def tearDown(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_reduce_picks_latest_time(self):
        reduced = reduce_mod.reduce_case(self.tmp, rho=1.0, stagnation_eps=0.05)
        # From the 100/ dir, not the 999.0 decoy in 0/.
        self.assertAlmostEqual(reduced["pressure_drop_pa"], 100.0, places=6)
        self.assertAlmostEqual(reduced["min_velocity_m_s"], 0.01, places=6)
        self.assertTrue(reduced["stagnation"])  # 0.01 < 0.05

    def test_write_then_stage_parses_exact_values(self):
        reduce_mod.write_cfd_results(self.tmp, rho=1.0, stagnation_eps=0.05)
        # stage_cfd.py's own parser is the oracle — proves the line format matches.
        parsed = stage_cfd._parse_cfd_results(self.tmp)
        self.assertIsNotNone(parsed, "stage_cfd could not parse the reducer's file")
        self.assertAlmostEqual(parsed["pressure_drop_pa"], 100.0, places=4)
        self.assertAlmostEqual(parsed["min_velocity_m_s"], 0.01, places=4)
        self.assertTrue(parsed["stagnation"])

    def test_no_stagnation_when_fast(self):
        with open(os.path.join(self.t100, "U"), "w") as fh:
            fh.write("internalField   uniform (3 0 0);\n")
        reduce_mod.write_cfd_results(self.tmp, rho=1.0, stagnation_eps=0.05)
        parsed = stage_cfd._parse_cfd_results(self.tmp)
        self.assertAlmostEqual(parsed["min_velocity_m_s"], 3.0, places=4)
        self.assertFalse(parsed["stagnation"])


if __name__ == "__main__":
    unittest.main()

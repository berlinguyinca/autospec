"""test_ccx_safety_wrapper.py — unit tests for the ccx→SAFETY_FACTOR wrapper.

The wrapper bridges real CalculiX .dat stress output to stage_fea.py's
``SAFETY_FACTOR <value>`` parse contract. The load-bearing assertion: the line the
wrapper appends is parsed by stage_fea.py's *real* ``_parse_safety_factor`` (imported
here, not re-implemented) to the value we derived. A sample real-ccx .dat fixture is
materialized at runtime so the test runs on a clean checkout.
"""

import importlib.util
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

import ccx_safety_wrapper as wrap  # noqa: E402
import stage_fea  # noqa: E402  (the real, unchanged stage parser is the oracle)


# A sample real-ccx .dat: header + a ``stresses`` block with two integration-point
# rows (elem, intpnt, sxx syy szz sxy sxz syz). Real ccx emits no SAFETY_FACTOR.
_SAMPLE_DAT = """\

 S T E P   1

 stresses (elem, integ.pnt.,sxx,syy,szz,sxy,sxz,syz) for set EALL and time  0.1000000E+01

         1   1  1.000000E+07  0.000000E+00  0.000000E+00  0.000000E+00  0.000000E+00  0.000000E+00
         1   2  2.000000E+07  0.000000E+00  0.000000E+00  0.000000E+00  0.000000E+00  0.000000E+00

 displacements (vx,vy,vz) for set NALL and time  0.1000000E+01

         5 -1.0E-04  0.0E+00  0.0E+00
"""


class TestVonMises(unittest.TestCase):
    def test_uniaxial_equals_axial_stress(self):
        # Pure uniaxial sxx → von-Mises == sxx.
        self.assertAlmostEqual(wrap.von_mises(10.0, 0, 0, 0, 0, 0), 10.0, places=6)

    def test_pure_shear(self):
        # Pure shear sxy=s → von-Mises == sqrt(3)*s.
        self.assertAlmostEqual(wrap.von_mises(0, 0, 0, 5.0, 0, 0),
                               math.sqrt(3.0) * 5.0, places=6)

    def test_hydrostatic_is_zero(self):
        self.assertAlmostEqual(wrap.von_mises(7, 7, 7, 0, 0, 0), 0.0, places=6)


class TestParseMaxMises(unittest.TestCase):
    def test_picks_max_over_rows(self):
        # Two rows: sxx=1e7 and sxx=2e7 (uniaxial) → max mises = 2e7.
        self.assertAlmostEqual(wrap.parse_max_mises(_SAMPLE_DAT), 2.0e7, places=1)

    def test_no_stress_rows_returns_none(self):
        self.assertIsNone(wrap.parse_max_mises("just\nheader\ntext\n"))


class TestSafetyDerivation(unittest.TestCase):
    def test_safety_factor_value(self):
        # yield 40 MPa = 4e7 Pa; max mises 2e7 Pa → SF = 2.0.
        sf = wrap.derive_safety_factor(2.0e7, 40.0)
        self.assertAlmostEqual(sf, 2.0, places=6)

    def test_zero_stress_is_large_finite(self):
        sf = wrap.derive_safety_factor(0.0, 40.0)
        self.assertGreater(sf, 1000.0)
        self.assertTrue(math.isfinite(sf))

    def test_material_yield_lookup_and_override(self):
        self.assertEqual(wrap.resolve_yield_mpa("PLA"), 60.0)
        self.assertEqual(wrap.resolve_yield_mpa("unknown"),
                         wrap.resolve_yield_mpa(None))
        os.environ["CCX_MATERIAL_YIELD_MPA"] = "123.0"
        try:
            self.assertEqual(wrap.resolve_yield_mpa("PLA"), 123.0)
        finally:
            del os.environ["CCX_MATERIAL_YIELD_MPA"]


class TestContractRoundTrip(unittest.TestCase):
    """The crux: wrapper output must be parsed by stage_fea.py's real parser."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="ccxwrap_")
        self.job_base = os.path.join(self.tmp, "fea_job")
        self.dat_path = self.job_base + ".dat"
        with open(self.dat_path, "w") as fh:
            fh.write(_SAMPLE_DAT)

    def tearDown(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_annotate_then_stage_parses_exact_value(self):
        appended = wrap.annotate_dat(self.dat_path, material="ABS")  # yield 40 MPa
        self.assertTrue(appended)
        # stage_fea.py's own parser is the oracle — proves the line format matches.
        parsed = stage_fea._parse_safety_factor(self.job_base)
        self.assertIsNotNone(parsed, "stage_fea could not parse the wrapper's line")
        # max mises 2e7 Pa, yield 40 MPa → SF = 2.0.
        self.assertAlmostEqual(parsed, 2.0, places=4)

    def test_no_stress_block_appends_nothing(self):
        with open(self.dat_path, "w") as fh:
            fh.write("header only, no stresses\n")
        appended = wrap.annotate_dat(self.dat_path, material="ABS")
        self.assertFalse(appended)
        self.assertIsNone(stage_fea._parse_safety_factor(self.job_base))


if __name__ == "__main__":
    unittest.main()

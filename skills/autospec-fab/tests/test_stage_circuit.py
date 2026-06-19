"""
test_stage_circuit.py — unittest for stage_circuit.py.

Tests:
  1. connected fixture      → status pass (every inlet reaches every target,
                               isolation intact, no relief on standard pump)
  2. broken-link fixture    → status fail + code_health disconnected_flow
                               (inlet_a cannot reach cavity_y via severed edge)
  3. leaked-isolation fixture → status fail (path between isolated_groups,
                               self_clamping false)
  4. relief-low-flow fixture  → status fail (pump_type low_flow + relief_nodes
                               present → relief/bleed forbidden)
  5. fragment shape         → stage="vacuum-circuit", status in {pass,fail,warn,skip},
                               keys: stage, status, detail, findings
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_SCRIPTS_DIR = os.path.normpath(os.path.join(_THIS_DIR, "..", "scripts"))
_FIXTURES_DIR = os.path.normpath(os.path.join(_THIS_DIR, "fixtures", "circuit"))

_STAGE_SCRIPT = os.path.join(_SCRIPTS_DIR, "stage_circuit.py")

# Reuse a small existing STL (any valid STL; circuit stage doesn't gate on geometry)
_SHARED_STL = os.path.normpath(
    os.path.join(_THIS_DIR, "fixtures", "port-good", "body.stl")
)

_CONNECTED_CIRCUIT    = os.path.join(_FIXTURES_DIR, "connected.json")
_BROKEN_CIRCUIT       = os.path.join(_FIXTURES_DIR, "broken-link.json")
_LEAKED_CIRCUIT       = os.path.join(_FIXTURES_DIR, "leaked-isolation.json")
_RELIEF_LF_CIRCUIT    = os.path.join(_FIXTURES_DIR, "relief-low-flow.json")


def _run_stage(circuit_path, extra_args=None):
    """Run stage_circuit.py and return (returncode, fragment_dict, stderr)."""
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
        out_path = tf.name
    try:
        cmd = [
            sys.executable, _STAGE_SCRIPT,
            "--in", _SHARED_STL,
            "--circuit", circuit_path,
            "--out", out_path,
        ]
        if extra_args:
            cmd.extend(extra_args)
        result = subprocess.run(cmd, capture_output=True, text=True)
        fragment = None
        if os.path.exists(out_path):
            with open(out_path) as fh:
                try:
                    fragment = json.load(fh)
                except json.JSONDecodeError:
                    pass
        return result.returncode, fragment, result.stderr
    finally:
        if os.path.exists(out_path):
            os.unlink(out_path)


def _finding_codes(fragment):
    """Return set of code values from fragment findings list."""
    return {f.get("code") for f in fragment.get("findings", [])}


# ---------------------------------------------------------------------------
# Fragment shape contract
# ---------------------------------------------------------------------------

class TestFragmentShape(unittest.TestCase):
    """Every invocation emits a valid release-gate stage-record fragment."""

    VALID_STATUSES = {"pass", "fail", "warn", "skip"}

    def _assert_valid_fragment(self, fragment, label=""):
        self.assertIsInstance(fragment, dict, f"{label} fragment is not a dict")
        for key in ("stage", "status", "detail", "findings"):
            self.assertIn(key, fragment, f"{label} fragment missing key '{key}'")
        self.assertIn(
            fragment["status"], self.VALID_STATUSES,
            f"{label} invalid status: {fragment['status']!r}",
        )
        self.assertEqual(
            fragment["stage"], "vacuum-circuit",
            f"{label} stage name must be 'vacuum-circuit'",
        )
        self.assertIsInstance(fragment["findings"], list,
                              f"{label} findings must be a list")

    def test_connected_fragment_shape(self):
        _, frag, _ = _run_stage(_CONNECTED_CIRCUIT)
        self.assertIsNotNone(frag, "no fragment emitted for connected circuit")
        self._assert_valid_fragment(frag, "connected")

    def test_broken_fragment_shape(self):
        _, frag, _ = _run_stage(_BROKEN_CIRCUIT)
        self.assertIsNotNone(frag, "no fragment emitted for broken-link circuit")
        self._assert_valid_fragment(frag, "broken-link")

    def test_leaked_fragment_shape(self):
        _, frag, _ = _run_stage(_LEAKED_CIRCUIT)
        self.assertIsNotNone(frag, "no fragment emitted for leaked-isolation circuit")
        self._assert_valid_fragment(frag, "leaked-isolation")

    def test_relief_lf_fragment_shape(self):
        _, frag, _ = _run_stage(_RELIEF_LF_CIRCUIT)
        self.assertIsNotNone(frag, "no fragment emitted for relief-low-flow circuit")
        self._assert_valid_fragment(frag, "relief-low-flow")


# ---------------------------------------------------------------------------
# Case 1: fully connected circuit → pass
# ---------------------------------------------------------------------------

class TestConnectedCircuit(unittest.TestCase):
    """Every inlet reaches every target, isolation intact → pass."""

    def test_connected_passes(self):
        rc, frag, stderr = _run_stage(_CONNECTED_CIRCUIT)
        self.assertEqual(rc, 0, f"harness error: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(
            frag["status"], "pass",
            f"expected pass, got {frag['status']!r}; detail={frag.get('detail')}",
        )

    def test_connected_no_disconnected_flow(self):
        _, frag, _ = _run_stage(_CONNECTED_CIRCUIT)
        self.assertIsNotNone(frag)
        codes = _finding_codes(frag)
        self.assertNotIn(
            "disconnected_flow", codes,
            "fully connected circuit must not produce disconnected_flow finding",
        )

    def test_connected_no_findings(self):
        _, frag, _ = _run_stage(_CONNECTED_CIRCUIT)
        self.assertIsNotNone(frag)
        self.assertEqual(
            frag["findings"], [],
            f"fully connected circuit must have empty findings, got: {frag['findings']}",
        )


# ---------------------------------------------------------------------------
# Case 2: broken inlet→gasket link → fail disconnected_flow
# ---------------------------------------------------------------------------

class TestBrokenLink(unittest.TestCase):
    """A severed edge leaves inlet_a unable to reach cavity_y → fail."""

    def test_broken_link_fails(self):
        rc, frag, stderr = _run_stage(_BROKEN_CIRCUIT)
        self.assertEqual(rc, 0, f"harness error: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(
            frag["status"], "fail",
            f"expected fail for broken-link, got {frag['status']!r}",
        )

    def test_broken_link_emits_disconnected_flow(self):
        _, frag, _ = _run_stage(_BROKEN_CIRCUIT)
        self.assertIsNotNone(frag)
        codes = _finding_codes(frag)
        self.assertIn(
            "disconnected_flow", codes,
            "broken inlet→target path must emit disconnected_flow finding",
        )

    def test_broken_link_detail_names_broken_pair(self):
        """Detail string must name the unreachable inlet→target pair."""
        _, frag, _ = _run_stage(_BROKEN_CIRCUIT)
        self.assertIsNotNone(frag)
        detail = frag.get("detail", "")
        # The fixture has inlet_a that cannot reach cavity_y
        self.assertIn(
            "inlet_a", detail,
            f"detail must name the broken inlet; got: {detail!r}",
        )


# ---------------------------------------------------------------------------
# Case 3: cross-circuit isolation leak → fail
# ---------------------------------------------------------------------------

class TestLeakedIsolation(unittest.TestCase):
    """Path between isolated_groups when self_clamping=false → fail."""

    def test_leaked_isolation_fails(self):
        rc, frag, stderr = _run_stage(_LEAKED_CIRCUIT)
        self.assertEqual(rc, 0, f"harness error: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(
            frag["status"], "fail",
            f"expected fail for leaked-isolation, got {frag['status']!r}",
        )

    def test_leaked_isolation_emits_finding(self):
        _, frag, _ = _run_stage(_LEAKED_CIRCUIT)
        self.assertIsNotNone(frag)
        # Must emit either disconnected_flow or circuit_isolation_violation
        codes = _finding_codes(frag)
        self.assertTrue(
            codes & {"disconnected_flow", "circuit_isolation_violation"},
            f"isolation leak must emit isolation finding; codes: {codes}",
        )

    def test_self_clamping_accepts_shared_input(self):
        """When self_clamping=true the stage must accept cross-group paths."""
        # Build a variant of leaked-isolation with self_clamping=true
        with open(_LEAKED_CIRCUIT) as fh:
            circuit = json.load(fh)
        circuit["self_clamping"] = True
        with tempfile.NamedTemporaryFile(
            suffix=".json", mode="w", delete=False
        ) as tf:
            json.dump(circuit, tf)
            tmp_circuit = tf.name
        try:
            rc, frag, stderr = _run_stage(tmp_circuit)
            self.assertEqual(rc, 0, f"harness error: {stderr}")
            self.assertIsNotNone(frag)
            # Isolation is waived; reachability still must pass (all nodes reachable)
            # In this fixture every inlet reaches every target via the bridge,
            # so overall status should be pass.
            self.assertEqual(
                frag["status"], "pass",
                f"self_clamping=true should accept shared-input branches; "
                f"got {frag['status']!r}; detail={frag.get('detail')}",
            )
        finally:
            os.unlink(tmp_circuit)


# ---------------------------------------------------------------------------
# Case 4: relief/bleed node on low-flow pump → fail
# ---------------------------------------------------------------------------

class TestReliefLowFlow(unittest.TestCase):
    """pump_type=low_flow + non-empty relief_nodes → fail."""

    def test_relief_low_flow_fails(self):
        rc, frag, stderr = _run_stage(_RELIEF_LF_CIRCUIT)
        self.assertEqual(rc, 0, f"harness error: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(
            frag["status"], "fail",
            f"expected fail for relief-low-flow, got {frag['status']!r}",
        )

    def test_relief_low_flow_finding_code(self):
        _, frag, _ = _run_stage(_RELIEF_LF_CIRCUIT)
        self.assertIsNotNone(frag)
        codes = _finding_codes(frag)
        self.assertTrue(
            codes & {"disconnected_flow", "relief_on_low_flow"},
            f"relief on low-flow must emit a finding; codes: {codes}",
        )

    def test_standard_pump_with_relief_passes_pump_check(self):
        """Relief nodes on a standard pump must NOT trigger the pump check."""
        with open(_RELIEF_LF_CIRCUIT) as fh:
            circuit = json.load(fh)
        circuit["pump_type"] = "standard"
        with tempfile.NamedTemporaryFile(
            suffix=".json", mode="w", delete=False
        ) as tf:
            json.dump(circuit, tf)
            tmp_circuit = tf.name
        try:
            rc, frag, stderr = _run_stage(tmp_circuit)
            self.assertEqual(rc, 0, f"harness error: {stderr}")
            self.assertIsNotNone(frag)
            # No pump-type violation; reachability is fine in this fixture
            codes = _finding_codes(frag)
            self.assertNotIn(
                "relief_on_low_flow", codes,
                "standard pump with relief nodes must NOT emit relief_on_low_flow",
            )
        finally:
            os.unlink(tmp_circuit)


class TestAbsentCircuitSkips(unittest.TestCase):
    """Engine sequences stages with only --in/--model/--out (no --circuit).

    A model with no vacuum circuit must DEGRADE the stage to a non-blocking
    skip, not hard-fail — otherwise the release-gate engine can never reach a
    green gate for a circuit-less model.
    """

    def _run_uniform(self, with_model=True):
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tf:
            out_path = tf.name
        try:
            cmd = [sys.executable, _STAGE_SCRIPT, "--in", _SHARED_STL,
                   "--out", out_path]
            if with_model:
                cmd.extend(["--model", _SHARED_STL])  # any path; unused
            result = subprocess.run(cmd, capture_output=True, text=True)
            frag = None
            if os.path.exists(out_path):
                with open(out_path) as fh:
                    frag = json.load(fh)
            return result.returncode, frag, result.stderr
        finally:
            if os.path.exists(out_path):
                os.unlink(out_path)

    def test_no_circuit_skips(self):
        rc, frag, stderr = self._run_uniform(with_model=False)
        self.assertEqual(rc, 0, f"harness error without --circuit: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(frag.get("status"), "skip")
        self.assertEqual(frag.get("stage"), "vacuum-circuit")

    def test_uniform_cli_with_model_skips(self):
        # Mirrors the engine's exact call: --in --model --out, no --circuit.
        rc, frag, stderr = self._run_uniform(with_model=True)
        self.assertEqual(rc, 0, f"harness error on uniform CLI: {stderr}")
        self.assertIsNotNone(frag)
        self.assertEqual(frag.get("status"), "skip")


if __name__ == "__main__":
    unittest.main()

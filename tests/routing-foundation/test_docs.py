from __future__ import annotations

import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from autospec_route_lib import load_routing_config  # noqa: E402


REFUSALS = {
    "ROUTING_CONFIG_MISSING",
    "ROUTING_CONFIG_INVALID",
    "ROUTING_DISCOVERY_FAILED",
    "ROUTING_CAPABILITY_INVALID",
    "ROUTING_CAPABILITY_STALE",
    "ROUTING_HARNESS_UNAVAILABLE",
    "ROUTING_CAPABILITY_UNAVAILABLE",
    "ROUTING_INDEPENDENCE_UNSATISFIED",
    "ROUTING_ADAPTER_UNSUPPORTED",
}


class RoutingDocumentationTests(unittest.TestCase):
    def test_operator_example_is_accepted_by_production_loader(self) -> None:
        config = load_routing_config(ROOT / "examples" / "routing.yml")
        self.assertEqual(config["version"], 1)
        self.assertEqual(config["fallback"]["mode"], "existing-routing")

    def test_config_reference_covers_control_and_rollback_contract(self) -> None:
        text = (ROOT / "docs" / "CONFIG_REFERENCE.md").read_text()
        for required in (
            "AUTOSPEC_ROUTING_CONFIG",
            "AUTOSPEC_SCHEMAS_DIR",
            "inferweave.discovery_url",
            "mac",
            "rtx6000",
            "existing-routing",
            "unset AUTOSPEC_ROUTING_CONFIG",
            "~/.autospec/routing.yml",
        ):
            self.assertIn(required, text)

    def test_api_reference_covers_cli_pi_payload_and_refusals(self) -> None:
        text = (ROOT / "docs" / "API_REFERENCE.md").read_text()
        for command in (
            "autospec-route.py validate",
            "autospec-route.py resolve",
            "autospec-route.py explain",
            "autospec-pi-dispatch.py",
            "--mode json",
            "--no-session",
            "INFERWEAVE_API_KEY",
            "exit 3",
        ):
            self.assertIn(command, text)
        for refusal in REFUSALS:
            self.assertIn(refusal, text)


if __name__ == "__main__":
    unittest.main()

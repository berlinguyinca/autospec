import copy
import json
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from autospec_route_lib import (  # noqa: E402
    RoutingError,
    load_capabilities,
    load_routing_config,
    validate_capabilities,
    validate_routing_config,
    validate_routing_config_path,
)


FIXTURES = Path(__file__).resolve().parent / "fixtures"
NOW = datetime(2026, 8, 21, 12, 0, 30, tzinfo=timezone.utc)


class ValidationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import yaml

        cls.routing = yaml.safe_load(
            (FIXTURES / "routing-valid.yml").read_text(encoding="utf-8")
        )
        cls.capabilities = json.loads(
            (FIXTURES / "capabilities-text.json").read_text(encoding="utf-8")
        )

    def assertRoutingError(self, reason, function, *args, **kwargs):
        with self.assertRaises(RoutingError) as caught:
            function(*args, **kwargs)
        self.assertEqual(caught.exception.reason, reason)
        return caught.exception

    def test_valid_configuration_and_capabilities_are_normalized(self):
        config = validate_routing_config(copy.deepcopy(self.routing))
        caps = validate_capabilities(
            copy.deepcopy(self.capabilities), now=NOW, maximum_age_seconds=60
        )
        self.assertEqual(config["version"], 1)
        self.assertEqual(caps["routes"][0]["id"], "qwen-text-48k-a")

    def test_unknown_nested_configuration_key_is_rejected(self):
        invalid = copy.deepcopy(self.routing)
        invalid["harnesses"]["pi"]["mdoel"] = "typo"
        self.assertRoutingError(
            "ROUTING_CONFIG_INVALID", validate_routing_config, invalid
        )

    def test_missing_inference_class_and_bad_independence_are_rejected(self):
        invalid = copy.deepcopy(self.routing)
        invalid["routes"]["execution"]["inference_class"] = "missing"
        self.assertRoutingError(
            "ROUTING_CONFIG_INVALID", validate_routing_config, invalid
        )
        invalid = copy.deepcopy(self.routing)
        invalid["routes"]["review"]["independent_from"] = "missing"
        self.assertRoutingError(
            "ROUTING_CONFIG_INVALID", validate_routing_config, invalid
        )

    def test_image_class_requires_exact_vision_node_allowlist(self):
        invalid = copy.deepcopy(self.routing)
        invalid["inference_classes"]["vision.standard"]["eligible_node_classes"] = [
            "mac",
            "rtx4090",
        ]
        self.assertRoutingError(
            "ROUTING_CONFIG_INVALID", validate_routing_config, invalid
        )

    def test_relative_override_path_is_rejected(self):
        self.assertRoutingError(
            "ROUTING_CONFIG_INVALID",
            validate_routing_config_path,
            Path("relative/routing.yml"),
            from_environment=True,
        )

    def test_capability_input_cannot_exceed_context_window(self):
        invalid = copy.deepcopy(self.capabilities)
        invalid["routes"][0]["max_input_tokens"] = 60001
        self.assertRoutingError(
            "ROUTING_CAPABILITY_INVALID",
            validate_capabilities,
            invalid,
            now=NOW,
            maximum_age_seconds=60,
        )

    def test_duplicate_capability_ids_invalidate_document(self):
        invalid = copy.deepcopy(self.capabilities)
        invalid["routes"].append(copy.deepcopy(invalid["routes"][0]))
        self.assertRoutingError(
            "ROUTING_CAPABILITY_INVALID",
            validate_capabilities,
            invalid,
            now=NOW,
            maximum_age_seconds=60,
        )

    def test_stale_capabilities_have_distinct_reason(self):
        self.assertRoutingError(
            "ROUTING_CAPABILITY_STALE",
            validate_capabilities,
            copy.deepcopy(self.capabilities),
            now=datetime(2026, 8, 21, 12, 2, tzinfo=timezone.utc),
            maximum_age_seconds=60,
        )

    def test_http_is_allowed_only_for_explicit_loopback_development(self):
        invalid = copy.deepcopy(self.capabilities)
        invalid["routes"][0]["endpoint"] = "http://inferweave.example/v1"
        self.assertRoutingError(
            "ROUTING_CAPABILITY_INVALID",
            validate_capabilities,
            invalid,
            now=NOW,
            maximum_age_seconds=60,
        )
        allowed = copy.deepcopy(self.capabilities)
        allowed["routes"][0]["endpoint"] = "http://127.0.0.1:8080/v1"
        validate_capabilities(
            allowed, now=NOW, maximum_age_seconds=60, allow_loopback_http=True
        )

    def test_loaders_enforce_size_bounds(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            huge = root / "huge.json"
            huge.write_text("x" * 1_048_577, encoding="utf-8")
            self.assertRoutingError(
                "ROUTING_CAPABILITY_INVALID",
                load_capabilities,
                huge,
                now=NOW,
                maximum_age_seconds=60,
            )

    def test_missing_configuration_requests_compatibility_fallback(self):
        missing = Path(tempfile.gettempdir()) / "autospec-routing-does-not-exist.yml"
        error = self.assertRoutingError(
            "ROUTING_CONFIG_MISSING", load_routing_config, missing
        )
        self.assertEqual(error.exit_code, 3)


if __name__ == "__main__":
    unittest.main()

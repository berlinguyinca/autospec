import copy
import json
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from autospec_route_lib import RoutingError, resolve_dispatch  # noqa: E402


FIXTURES = Path(__file__).resolve().parent / "fixtures"


class ResolverTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import yaml

        cls.config = yaml.safe_load(
            (FIXTURES / "routing-valid.yml").read_text(encoding="utf-8")
        )
        cls.text = json.loads(
            (FIXTURES / "capabilities-text.json").read_text(encoding="utf-8")
        )
        cls.vision = json.loads(
            (FIXTURES / "capabilities-vision.json").read_text(encoding="utf-8")
        )

    def assertReason(self, reason, function, *args, **kwargs):
        with self.assertRaises(RoutingError) as caught:
            function(*args, **kwargs)
        self.assertEqual(caught.exception.reason, reason)

    def test_selects_harness_and_inference_independently(self):
        envelope = resolve_dispatch(
            copy.deepcopy(self.config),
            copy.deepcopy(self.text),
            "execution",
            available_harnesses={"pi", "codex"},
        )
        self.assertEqual(envelope["harness"]["id"], "pi")
        self.assertEqual(envelope["inference"]["route_id"], "qwen-text-48k-a")

    def test_selects_lowest_queue_then_smallest_sufficient_route(self):
        capabilities = copy.deepcopy(self.text)
        base = capabilities["routes"][0]
        larger = copy.deepcopy(base)
        larger.update(id="larger", max_input_tokens=72000, context_window=80000)
        slower = copy.deepcopy(base)
        slower.update(id="slower", queue_seconds=20)
        capabilities["routes"] = [slower, larger, base]
        envelope = resolve_dispatch(
            self.config, capabilities, "execution", available_harnesses={"pi"}
        )
        self.assertEqual(envelope["inference"]["route_id"], "qwen-text-48k-a")

    def test_non_opportunistic_route_wins_before_queue_order(self):
        capabilities = copy.deepcopy(self.text)
        rare = copy.deepcopy(capabilities["routes"][0])
        rare.update(id="h100-idle", node_class="h100", opportunistic=True, queue_seconds=0)
        capabilities["routes"].insert(0, rare)
        envelope = resolve_dispatch(
            self.config, capabilities, "execution", available_harnesses={"pi"}
        )
        self.assertEqual(envelope["inference"]["route_id"], "qwen-text-48k-a")

    def test_text_work_does_not_require_scarce_nodes(self):
        envelope = resolve_dispatch(
            self.config, self.text, "planning", available_harnesses={"pi"}
        )
        self.assertEqual(envelope["inference"]["node_class"], "dual-1080ti")

    def test_candidate_context_must_fit_input_and_reserved_output(self):
        capabilities = copy.deepcopy(self.text)
        capabilities["routes"][0]["context_window"] = 59000
        self.assertReason(
            "ROUTING_CAPABILITY_UNAVAILABLE",
            resolve_dispatch,
            self.config,
            capabilities,
            "execution",
            available_harnesses={"pi"},
        )

    def test_vision_selects_mac_and_never_downgrades_to_text(self):
        envelope = resolve_dispatch(
            self.config, self.vision, "visual_qa", available_harnesses={"pi"}
        )
        self.assertEqual(envelope["inference"]["node_class"], "mac")
        self.assertReason(
            "ROUTING_CAPABILITY_UNAVAILABLE",
            resolve_dispatch,
            self.config,
            self.text,
            "visual_qa",
            available_harnesses={"pi"},
        )

    def test_protocol_must_be_supported_by_selected_harness(self):
        config = copy.deepcopy(self.config)
        config["harnesses"]["pi"]["provider_protocols"] = ["native"]
        self.assertReason(
            "ROUTING_ADAPTER_UNSUPPORTED",
            resolve_dispatch,
            config,
            self.text,
            "execution",
            available_harnesses={"pi"},
        )

    def test_local_only_excludes_cloud_candidates(self):
        config = copy.deepcopy(self.config)
        config["inferweave"]["local_only"] = True
        capabilities = copy.deepcopy(self.text)
        capabilities["routes"][0]["local"] = False
        self.assertReason(
            "ROUTING_CAPABILITY_UNAVAILABLE",
            resolve_dispatch,
            config,
            capabilities,
            "execution",
            available_harnesses={"pi"},
        )

    def test_review_requires_independent_harness_route_and_strength(self):
        proposer = {
            "harness": {"id": "pi"},
            "inference": {"route_id": "qwen-text-48k-a", "strength": 20},
        }
        capabilities = copy.deepcopy(self.text)
        reviewer = copy.deepcopy(capabilities["routes"][0])
        reviewer.update(id="reviewer-route", strength=25)
        capabilities["routes"].append(reviewer)
        envelope = resolve_dispatch(
            self.config,
            capabilities,
            "review",
            proposer=proposer,
            available_harnesses={"pi", "codex"},
        )
        self.assertEqual(envelope["harness"]["id"], "codex")
        self.assertEqual(envelope["inference"]["route_id"], "reviewer-route")

    def test_review_refuses_when_independence_cannot_be_satisfied(self):
        proposer = {
            "harness": {"id": "pi"},
            "inference": {"route_id": "qwen-text-48k-a", "strength": 20},
        }
        self.assertReason(
            "ROUTING_INDEPENDENCE_UNSATISFIED",
            resolve_dispatch,
            self.config,
            self.text,
            "review",
            proposer=proposer,
            available_harnesses={"pi"},
        )

    def test_identical_inputs_produce_byte_identical_envelopes(self):
        first = resolve_dispatch(
            self.config, self.text, "execution", available_harnesses={"pi"}
        )
        second = resolve_dispatch(
            self.config, self.text, "execution", available_harnesses={"pi"}
        )
        self.assertEqual(
            json.dumps(first, sort_keys=True, separators=(",", ":")),
            json.dumps(second, sort_keys=True, separators=(",", ":")),
        )


if __name__ == "__main__":
    unittest.main()

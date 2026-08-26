import copy
import json
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCHEMAS = ROOT / "schemas"
FIXTURES = Path(__file__).resolve().parent / "fixtures"


def load_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def validate(instance, schema):
    try:
        import jsonschema
    except ImportError:
        missing = set(schema.get("required", [])) - set(instance)
        if missing:
            raise AssertionError(f"missing required keys: {sorted(missing)}")
        if schema.get("additionalProperties") is False:
            unknown = set(instance) - set(schema.get("properties", {}))
            if unknown:
                raise AssertionError(f"unknown keys: {sorted(unknown)}")
        return
    jsonschema.Draft202012Validator(schema).validate(instance)


class RoutingSchemaTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.routing_schema = load_json(SCHEMAS / "autospec-routing-v1.schema.json")
        cls.capabilities_schema = load_json(
            SCHEMAS / "inferweave-capabilities-v1.schema.json"
        )
        cls.envelope_schema = load_json(
            SCHEMAS / "autospec-dispatch-envelope-v1.schema.json"
        )
        try:
            import yaml
        except ImportError as exc:
            raise unittest.SkipTest(f"PyYAML unavailable: {exc}")
        cls.routing = yaml.safe_load(
            (FIXTURES / "routing-valid.yml").read_text(encoding="utf-8")
        )
        cls.text_capabilities = load_json(FIXTURES / "capabilities-text.json")
        cls.vision_capabilities = load_json(FIXTURES / "capabilities-vision.json")

    def test_canonical_routing_fixture_matches_schema(self):
        validate(self.routing, self.routing_schema)

    def test_text_and_vision_capability_fixtures_match_schema(self):
        validate(self.text_capabilities, self.capabilities_schema)
        validate(self.vision_capabilities, self.capabilities_schema)

    def test_unknown_routing_key_is_rejected(self):
        invalid = copy.deepcopy(self.routing)
        invalid["unexpected"] = True
        with self.assertRaises(Exception):
            validate(invalid, self.routing_schema)

    def test_vision_route_rejects_unapproved_node_class(self):
        candidate = self.capabilities_schema["$defs"]["candidate"]
        allowed = candidate["allOf"][0]["then"]["properties"]["node_class"]["enum"]
        self.assertEqual(allowed, ["mac", "rtx6000"])

    def test_success_envelope_fixture_matches_schema(self):
        envelope = load_json(FIXTURES / "dispatch-envelope.json")
        validate(envelope, self.envelope_schema)


if __name__ == "__main__":
    unittest.main()

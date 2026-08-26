import copy
import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCHEMAS = ROOT / "schemas"
FIXTURES = Path(__file__).resolve().parent / "fixtures"
FILES = {
    "spec": ("autospec-spec-v1.schema.json", "spec.json"),
    "implementation": ("autospec-implementation-handoff-v1.schema.json", "implementation-handoff.json"),
    "review": ("autospec-review-handoff-v1.schema.json", "review-handoff.json"),
    "result": ("autospec-agent-handoff-result-v1.schema.json", "agent-result.json"),
    "closeout": ("autospec-implementation-closeout-v1.schema.json", "implementation-closeout.json"),
}


def load_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def validate(instance, schema):
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        schema_path = root / "schema.json"
        instance_path = root / "instance.json"
        schema_path.write_text(json.dumps(schema), encoding="utf-8")
        instance_path.write_text(json.dumps(instance), encoding="utf-8")
        result = subprocess.run(
            ["ajv", "validate", "--spec=draft2020", "-s", str(schema_path), "-d", str(instance_path)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            raise AssertionError(result.stderr or result.stdout)


class HandoffSchemaTests(unittest.TestCase):
    def schema(self, kind):
        return load_json(SCHEMAS / FILES[kind][0])

    def fixture(self, kind):
        return load_json(FIXTURES / FILES[kind][1])

    def test_valid_fixtures_match_schemas(self):
        for kind in FILES:
            with self.subTest(kind=kind):
                validate(self.fixture(kind), self.schema(kind))

    def test_approved_spec_rejects_material_questions(self):
        spec = self.fixture("spec")
        spec["material_questions"] = ["Which persistence backend?"]
        with self.assertRaises(Exception):
            validate(spec, self.schema("spec"))

    def test_proposal_does_not_claim_repository_critique_evidence(self):
        spec = self.fixture("spec")
        spec["status"] = "proposal"
        spec["sources"] = []
        spec["planning_evidence"] = []
        validate(spec, self.schema("spec"))

    def test_unknown_fields_are_rejected(self):
        for kind in FILES:
            with self.subTest(kind=kind):
                artifact = copy.deepcopy(self.fixture(kind))
                artifact["conversation"] = "not part of the protocol"
                with self.assertRaises(Exception):
                    validate(artifact, self.schema(kind))

    def test_result_error_status_requires_error_category(self):
        result = self.fixture("result")
        result["status"] = "error"
        result["error_category"] = None
        with self.assertRaises(Exception):
            validate(result, self.schema("result"))


if __name__ == "__main__":
    unittest.main()

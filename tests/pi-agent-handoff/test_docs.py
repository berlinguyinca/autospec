import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


class HandoffDocumentationTests(unittest.TestCase):
    def test_api_reference_documents_every_command_and_stable_refusal(self):
        api = (ROOT / "docs" / "API_REFERENCE.md").read_text(encoding="utf-8")
        for command in ("validate", "reconcile-spec", "implementation", "review", "accept-result"):
            self.assertIn(command, api)
        for refusal in (
            "HANDOFF_SCHEMA_INVALID", "HANDOFF_LINEAGE_MISMATCH", "HANDOFF_SCOPE_INVALID",
            "HANDOFF_BRIDGE_DISABLED", "HANDOFF_BRIDGE_UNAVAILABLE",
            "HANDOFF_AGENT_OUTPUT_INVALID", "HANDOFF_INDEPENDENCE_UNSATISFIED",
        ):
            self.assertIn(refusal, api)

    def test_operator_docs_cover_opt_in_rollback_and_authority(self):
        config = (ROOT / "docs" / "CONFIG_REFERENCE.md").read_text(encoding="utf-8")
        manual = (ROOT / "docs" / "USER_MANUAL.md").read_text(encoding="utf-8")
        self.assertIn("AUTOSPEC_PI_HANDOFF_CONFIG", config)
        self.assertIn("examples/pi-agent-handoff.yml", config)
        self.assertIn("extension-free", config.lower())
        self.assertIn("AutoSpec retains", manual)
        self.assertIn("AskClaude", manual)
        self.assertIn("AskCodex", manual)

    def test_disabled_example_is_accepted_without_attempting_dispatch(self):
        with self.subTest("production config loader"):
            result = subprocess.run(
                ["python3", str(ROOT / "scripts" / "autospec-pi-bridge-dispatch.py"),
                 "--config", str(ROOT / "examples" / "pi-agent-handoff.yml"),
                 "--role", "intent_planner", "--input", str(ROOT / "tests" / "pi-agent-handoff" / "fixtures" / "spec.json"),
                 "--output", "/dev/null", "--repo", str(ROOT)],
                text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
            )
            self.assertEqual(result.returncode, 3)
            self.assertIn("HANDOFF_BRIDGE_DISABLED", result.stderr)


if __name__ == "__main__":
    unittest.main()

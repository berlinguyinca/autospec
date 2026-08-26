import hashlib
import json
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
ADAPTER = ROOT / "scripts" / "autospec-pi-bridge-dispatch.py"
FIXTURES = Path(__file__).resolve().parent / "fixtures"


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def digest(value):
    return "sha256:" + hashlib.sha256(canonical(value).encode()).hexdigest()


class PiBridgeAdapterTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.capture = self.root / "capture.json"
        self.prompt_capture = self.root / "prompt.json"
        self.input = self.root / "input.json"
        self.input_value = json.loads((FIXTURES / "spec.json").read_text(encoding="utf-8"))
        self.input.write_text(json.dumps(self.input_value), encoding="utf-8")
        self.output = self.root / "output.json"
        self.claude = self.package("pi-claude-bridge", "0.7.0")
        self.codex = self.package("@estebanforge/pi-ask-codex", "1.0.3")
        self.config = self.root / "config.yml"
        self.write_config()

    def tearDown(self):
        self.temp.cleanup()

    def package(self, name, version):
        root = self.root / name.replace("/", "-").lstrip("@")
        (root / "src").mkdir(parents=True)
        (root / "src" / "index.ts").write_text("export default function extension() {}\n", encoding="utf-8")
        (root / "package.json").write_text(
            json.dumps({"name": name, "version": version, "pi": {"extensions": ["./src/index.ts"]}}),
            encoding="utf-8",
        )
        return root

    def write_config(self, enabled=True, claude_source="npm:pi-claude-bridge@0.7.0"):
        self.config.write_text(
            f"""version: 1
enabled: {str(enabled).lower()}
orchestrator:
  provider: ollama
  model: glm-4.7-flash
bridges:
  intent_planner:
    package: {claude_source}
    tool: AskClaude
    provider_family: anthropic
    model: opus
    reasoning_effort: high
  repository_critic:
    package: npm:@estebanforge/pi-ask-codex@1.0.3
    tool: AskCodex
    provider_family: openai
    model: full
    reasoning_effort: high
policy:
  max_parallel: 2
  recursive_delegation: false
  require_isolated_planning_sessions: true
""",
            encoding="utf-8",
        )

    def result(self, role, bridge, provider):
        value = json.loads((FIXTURES / "agent-result.json").read_text(encoding="utf-8"))
        value["role"] = role
        value["producer"]["bridge"] = bridge
        value["producer"]["provider_family"] = provider
        value["inputs"] = [{"artifact_id": self.input_value["artifact_id"], "digest": digest(self.input_value)}]
        return value

    def write_pi(self, role="intent_planner", bridge="AskClaude", provider="anthropic", malformed=False, exit_status=0):
        payload = "not-json" if malformed else canonical(self.result(role, bridge, provider))
        script = self.bin / "pi"
        script.write_text(
            "#!/usr/bin/env python3\n"
            "import json, pathlib, sys\n"
            f"claude={str(self.claude)!r}\n"
            f"codex={str(self.codex)!r}\n"
            "if len(sys.argv) > 1 and sys.argv[1] == 'list':\n"
            " print('User packages:')\n"
            " print('  npm:pi-claude-bridge@0.7.0')\n"
            " print('    ' + claude)\n"
            " print('  npm:@estebanforge/pi-ask-codex@1.0.3')\n"
            " print('    ' + codex)\n"
            " raise SystemExit(0)\n"
            f"pathlib.Path({str(self.capture)!r}).write_text(json.dumps(sys.argv[1:]))\n"
            "prompt_arg = next(item for item in sys.argv[1:] if item.startswith('@'))\n"
            f"pathlib.Path({str(self.prompt_capture)!r}).write_text(pathlib.Path(prompt_arg[1:]).read_text())\n"
            f"message={payload!r}\n"
            "print(json.dumps({'type':'message_end','message':{'role':'assistant','content':[{'type':'text','text':message}],'usage':{'input':12,'output':3}}}))\n"
            f"raise SystemExit({exit_status})\n",
            encoding="utf-8",
        )
        script.chmod(script.stat().st_mode | stat.S_IXUSR)

    def run_adapter(self, role, env=None):
        process_env = os.environ.copy()
        process_env["PATH"] = f"{self.bin}{os.pathsep}{process_env['PATH']}"
        process_env["BRIDGE_TEST_SECRET"] = "super-secret-value"
        if env:
            process_env.update(env)
        return subprocess.run(
            ["python3", str(ADAPTER), "--config", str(self.config), "--role", role, "--input", str(self.input), "--output", str(self.output), "--repo", str(ROOT)],
            cwd=ROOT,
            env=process_env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def test_claude_planner_loads_only_pinned_extension_and_requests_isolated_read_mode(self):
        self.write_pi()
        result = self.run_adapter("intent_planner")
        self.assertEqual(result.returncode, 0, result.stderr)
        argv = json.loads(self.capture.read_text(encoding="utf-8"))
        self.assertIn("--no-extensions", argv)
        self.assertEqual(argv.count("--extension"), 1)
        extension = argv[argv.index("--extension") + 1]
        self.assertEqual(extension, str((self.claude / "src" / "index.ts").resolve()))
        request = json.loads(self.prompt_capture.read_text(encoding="utf-8"))
        self.assertEqual(request["tool"], "AskClaude")
        self.assertEqual(request["arguments"]["mode"], "read")
        self.assertTrue(request["arguments"]["isolated"])
        delegated = json.loads(request["arguments"]["prompt"])
        self.assertEqual(delegated["output_contract"]["proposed_artifact"], "autospec-spec-v1 status=proposal")
        self.assertEqual(delegated["output_contract"]["planning_evidence"], [])
        self.assertEqual(json.loads(self.output.read_text())["role"], "intent_planner")

    def test_codex_critic_requests_isolated_read_only_sandbox(self):
        self.write_pi(role="repository_critic", bridge="AskCodex", provider="openai")
        result = self.run_adapter("repository_critic")
        self.assertEqual(result.returncode, 0, result.stderr)
        request = json.loads(self.prompt_capture.read_text(encoding="utf-8"))
        self.assertEqual(request["tool"], "AskCodex")
        self.assertEqual(request["arguments"]["sandbox"], "read-only")
        self.assertNotIn("sessionId", request["arguments"])
        self.assertEqual(request["arguments"]["cwd"], str(ROOT))
        delegated = json.loads(request["arguments"]["prompt"])
        self.assertEqual(delegated["output_contract"]["proposed_artifact"], None)
        self.assertIn("repository paths and symbols", delegated["output_contract"]["findings"])

    def test_missing_unpinned_and_disabled_bridges_fail_closed(self):
        self.write_pi()
        self.write_config(claude_source="npm:pi-claude-bridge")
        result = self.run_adapter("intent_planner")
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_BRIDGE_UNAVAILABLE", result.stderr)
        self.write_config(enabled=False)
        result = self.run_adapter("intent_planner")
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_BRIDGE_DISABLED", result.stderr)

    def test_malformed_agent_output_and_lineage_mismatch_are_rejected(self):
        self.write_pi(malformed=True)
        result = self.run_adapter("intent_planner")
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_AGENT_OUTPUT_INVALID", result.stderr)
        self.write_pi(role="repository_critic", bridge="AskClaude", provider="anthropic")
        result = self.run_adapter("intent_planner")
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_LINEAGE_MISMATCH", result.stderr)

    def test_secret_never_appears_in_argv_prompt_or_output(self):
        self.write_pi()
        result = self.run_adapter("intent_planner")
        self.assertEqual(result.returncode, 0, result.stderr)
        combined = self.capture.read_text() + self.prompt_capture.read_text() + self.output.read_text() + result.stdout + result.stderr
        self.assertNotIn("super-secret-value", combined)


if __name__ == "__main__":
    unittest.main()

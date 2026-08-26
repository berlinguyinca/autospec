import json
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
ADAPTER = ROOT / "scripts" / "autospec-pi-dispatch.py"
FIXTURES = Path(__file__).resolve().parent / "fixtures"


class PiAdapterTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.capture = self.root / "argv.json"
        self.models = self.root / "models.json"
        self.prompt = self.root / "prompt.md"
        self.prompt.write_text("Implement the routed task.", encoding="utf-8")
        self.envelope = json.loads(
            (FIXTURES / "dispatch-envelope.json").read_text(encoding="utf-8")
        )
        self.envelope["harness"]["api_key_env"] = "INFERWEAVE_API_KEY"

    def tearDown(self):
        self.temp.cleanup()

    def write_pi(self, body: str, exit_status: int = 0):
        script = self.bin / "pi"
        script.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, pathlib, sys\n"
            f"pathlib.Path({str(self.capture)!r}).write_text(json.dumps(sys.argv[1:]))\n"
            f"pathlib.Path({str(self.models)!r}).write_text((pathlib.Path(os.environ['PI_CODING_AGENT_DIR']) / 'models.json').read_text())\n"
            f"{body}\n"
            f"raise SystemExit({exit_status})\n",
            encoding="utf-8",
        )
        script.chmod(script.stat().st_mode | stat.S_IXUSR)

    def run_adapter(self, envelope=None, env=None):
        envelope_path = self.root / "envelope.json"
        envelope_path.write_text(json.dumps(envelope or self.envelope), encoding="utf-8")
        process_env = os.environ.copy()
        process_env["PATH"] = f"{self.bin}{os.pathsep}{process_env['PATH']}"
        process_env["INFERWEAVE_API_KEY"] = "super-secret-value"
        if env:
            process_env.update(env)
        return subprocess.run(
            [
                "python3",
                str(ADAPTER),
                "--envelope",
                str(envelope_path),
                "--prompt-file",
                str(self.prompt),
            ],
            cwd=ROOT,
            env=process_env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def test_invokes_pi_json_mode_with_ephemeral_restricted_context(self):
        self.write_pi(
            "print(json.dumps({'type':'message_end','message':{'role':'assistant','content':[{'type':'text','text':'done'}],'usage':{'input':12,'output':3}}}))"
        )
        result = self.run_adapter()
        self.assertEqual(result.returncode, 0, result.stderr)
        argv = json.loads(self.capture.read_text(encoding="utf-8"))
        self.assertIn("--mode", argv)
        self.assertIn("json", argv)
        self.assertIn("--print", argv)
        self.assertIn("--no-session", argv)
        self.assertIn("--no-skills", argv)
        self.assertIn("--no-extensions", argv)
        self.assertNotIn("task", " ".join(argv))
        self.assertIn(f"@{self.prompt.resolve()}", argv)

    def test_ephemeral_model_config_contains_endpoint_model_and_env_reference_not_secret(self):
        self.write_pi("print(json.dumps({'type':'result','message':'ok'}))")
        result = self.run_adapter()
        self.assertEqual(result.returncode, 0, result.stderr)
        model_config = self.models.read_text(encoding="utf-8")
        self.assertIn("https://inferweave.example/v1", model_config)
        self.assertIn("qwen3.8-27b-48k", model_config)
        self.assertIn("INFERWEAVE_API_KEY", model_config)
        self.assertNotIn("super-secret-value", model_config)
        self.assertNotIn("super-secret-value", self.capture.read_text(encoding="utf-8"))

    def test_normalizes_final_message_and_usage(self):
        self.write_pi(
            "print(json.dumps({'type':'message_end','message':{'role':'assistant','content':[{'type':'text','text':'first '},{'type':'text','text':'result'}],'usage':{'input':120,'output':33,'cacheRead':40,'cacheWrite':2}}}))"
        )
        result = self.run_adapter()
        payload = json.loads(result.stdout)
        self.assertEqual(payload["message"], "first result")
        self.assertEqual(
            payload["usage"],
            {"input_tokens": 120, "output_tokens": 33, "cached_tokens": 40, "cache_write_tokens": 2},
        )

    def test_rejects_wrong_harness_and_unsupported_protocol(self):
        wrong = json.loads(json.dumps(self.envelope))
        wrong["harness"]["id"] = "codex"
        result = self.run_adapter(wrong)
        self.assertEqual(result.returncode, 3)
        self.assertIn("ROUTING_ADAPTER_UNSUPPORTED", result.stderr)
        wrong = json.loads(json.dumps(self.envelope))
        wrong["inference"]["protocol"] = "native"
        result = self.run_adapter(wrong)
        self.assertEqual(result.returncode, 3)

    def test_rejects_malformed_jsonl_and_missing_pi(self):
        self.write_pi("print('not-json')")
        result = self.run_adapter()
        self.assertEqual(result.returncode, 1)
        self.assertIn("malformed Pi JSONL", result.stderr)
        result = self.run_adapter(env={"PATH": "/usr/bin:/bin"})
        self.assertEqual(result.returncode, 2)

    def test_preserves_nonzero_child_status(self):
        self.write_pi("print(json.dumps({'type':'result','message':'failed'}))", exit_status=7)
        result = self.run_adapter()
        self.assertEqual(result.returncode, 7)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["child_exit_status"], 7)


if __name__ == "__main__":
    unittest.main()

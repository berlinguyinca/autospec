import json
import os
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "scripts" / "autospec-route.py"
FIXTURES = Path(__file__).resolve().parent / "fixtures"


class RoutingCliTests(unittest.TestCase):
    def run_cli(self, *args):
        return subprocess.run(
            ["python3", str(CLI), *args],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def test_resolve_emits_json_envelope(self):
        result = self.run_cli(
            "resolve",
            "--config",
            str(FIXTURES / "routing-valid.yml"),
            "--capabilities",
            str(FIXTURES / "capabilities-text.json"),
            "--kind",
            "execution",
            "--available-harness",
            "pi",
            "--now",
            "2026-08-21T12:00:30Z",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["harness"]["id"], "pi")

    def test_explain_formats_the_same_decision(self):
        args = (
            "--config",
            str(FIXTURES / "routing-valid.yml"),
            "--capabilities",
            str(FIXTURES / "capabilities-text.json"),
            "--kind",
            "execution",
            "--available-harness",
            "pi",
            "--now",
            "2026-08-21T12:00:30Z",
        )
        resolved = json.loads(self.run_cli("resolve", *args).stdout)
        explained = self.run_cli("explain", *args)
        self.assertEqual(explained.returncode, 0, explained.stderr)
        self.assertIn(resolved["dispatch_id"], explained.stdout)
        self.assertIn("qwen-text-48k-a", explained.stdout)

    def test_missing_config_emits_fallback_and_exit_three(self):
        result = self.run_cli(
            "resolve",
            "--config",
            "/tmp/autospec-routing-missing.yml",
            "--capabilities",
            str(FIXTURES / "capabilities-text.json"),
            "--kind",
            "execution",
        )
        self.assertEqual(result.returncode, 3)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["reason"], "ROUTING_CONFIG_MISSING")

    def test_environment_override_must_be_absolute(self):
        env = os.environ.copy()
        env["AUTOSPEC_ROUTING_CONFIG"] = "relative.yml"
        result = subprocess.run(
            ["python3", str(CLI), "validate"],
            cwd=ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("ROUTING_CONFIG_INVALID", result.stderr)

    def test_resolve_fetches_one_bounded_capability_document(self):
        capability_bytes = (FIXTURES / "capabilities-text.json").read_bytes()

        class Handler(BaseHTTPRequestHandler):
            calls = 0

            def do_GET(self):
                type(self).calls += 1
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(capability_bytes)))
                self.end_headers()
                self.wfile.write(capability_bytes)

            def log_message(self, *_args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as temp:
                config = (FIXTURES / "routing-valid.yml").read_text(encoding="utf-8")
                config = config.replace(
                    "https://inferweave.example/v1/capabilities",
                    f"http://127.0.0.1:{server.server_port}/v1/capabilities",
                ).replace("  local_only: false", "  local_only: false\n  allow_loopback_http: true")
                path = Path(temp) / "routing.yml"
                path.write_text(config, encoding="utf-8")
                result = self.run_cli(
                    "resolve",
                    "--config",
                    str(path),
                    "--kind",
                    "execution",
                    "--available-harness",
                    "pi",
                    "--now",
                    "2026-08-21T12:00:30Z",
                )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(Handler.calls, 1)
        finally:
            server.shutdown()
            thread.join()
            server.server_close()

    def test_discovery_redirect_requests_existing_fallback(self):
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(302)
                self.send_header("Location", "http://127.0.0.1/elsewhere")
                self.end_headers()

            def log_message(self, *_args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as temp:
                config = (FIXTURES / "routing-valid.yml").read_text(encoding="utf-8")
                config = config.replace(
                    "https://inferweave.example/v1/capabilities",
                    f"http://127.0.0.1:{server.server_port}/v1/capabilities",
                ).replace("  local_only: false", "  local_only: false\n  allow_loopback_http: true")
                path = Path(temp) / "routing.yml"
                path.write_text(config, encoding="utf-8")
                result = self.run_cli(
                    "resolve", "--config", str(path), "--kind", "execution"
                )
            self.assertEqual(result.returncode, 3)
            self.assertEqual(json.loads(result.stdout)["reason"], "ROUTING_DISCOVERY_FAILED")
        finally:
            server.shutdown()
            thread.join()
            server.server_close()


if __name__ == "__main__":
    unittest.main()

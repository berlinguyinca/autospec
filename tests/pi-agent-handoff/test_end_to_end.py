import hashlib
import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "scripts" / "autospec-handoff.py"
FIXTURES = Path(__file__).resolve().parent / "fixtures"


def load(name):
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


def digest(value):
    data = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(data).hexdigest()


class EndToEndHandoffTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        (self.repo / "scripts").mkdir(parents=True)
        (self.repo / "scripts" / "existing.py").write_text("def existing_symbol():\n    return True\n", encoding="utf-8")

    def tearDown(self):
        self.temp.cleanup()

    def write(self, name, value):
        path = self.root / name
        path.write_text(json.dumps(value), encoding="utf-8")
        return path

    def command(self, *args):
        return subprocess.run(["python3", str(CLI), *map(str, args)], cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)

    def test_isolated_planning_reaches_independent_review_with_exact_lineage(self):
        candidate = load("spec.json")
        candidate["status"] = "proposal"
        candidate["affected_surfaces"] = [{"path": "scripts/existing.py", "state": "existing", "symbols": ["existing_symbol"]}]
        proposal = load("agent-result.json")
        proposal["artifact_id"] = "sha256:" + "a" * 64
        proposal["role"] = "intent_planner"
        proposal["producer"].update({"bridge": "AskClaude", "provider_family": "anthropic"})
        proposal["proposed_artifact"] = candidate
        critique = load("agent-result.json")
        critique["artifact_id"] = "sha256:" + "b" * 64
        critique["role"] = "repository_critic"
        critique["producer"].update({"bridge": "AskCodex", "provider_family": "openai"})
        spec_path = self.root / "approved-spec.json"
        result = self.command("reconcile-spec", "--proposal", self.write("proposal.json", proposal), "--critique", self.write("critique.json", critique), "--repo", self.repo, "--output", spec_path)
        self.assertEqual(result.returncode, 0, result.stderr)
        spec = json.loads(spec_path.read_text())

        issue = {
            "number": 901, "title": "Implement typed handoffs", "branch": "feat/pi-agent-handoffs", "worktree": ".", "claim_generation": 1,
            "allowed_read_paths": ["scripts/", "schemas/"], "allowed_write_paths": ["scripts/autospec-handoff.py", "tests/pi-agent-handoff/"],
            "selected_acceptance_criteria": ["AC-1"], "interfaces": ["scripts/autospec-handoff.py:main"],
            "route": {"harness": "codex", "model": "gpt-5.5", "reasoning_effort": "medium"}
        }
        implementation_path = self.root / "implementation.json"
        result = self.command("implementation", "--spec", spec_path, "--issue", self.write("issue.json", issue), "--output", implementation_path)
        self.assertEqual(result.returncode, 0, result.stderr)
        implementation = json.loads(implementation_path.read_text())

        closeout = {
            "source_implementation": {"artifact_id": implementation["artifact_id"], "digest": digest(implementation)},
            "changed_paths": ["scripts/autospec-handoff.py", "tests/pi-agent-handoff/test_end_to_end.py"],
            "claims": load("review-handoff.json")["closeout"]["claims"],
            "checks": load("review-handoff.json")["checks"]
        }
        review_path = self.root / "review.json"
        result = self.command("review", "--implementation", implementation_path, "--closeout", self.write("closeout.json", closeout), "--base", "1" * 40, "--head", "2" * 40, "--output", review_path)
        self.assertEqual(result.returncode, 0, result.stderr)
        review = json.loads(review_path.read_text())

        verdict = load("agent-result.json")
        verdict["role"] = "reviewer"
        verdict["producer"].update({"bridge": "AskClaude", "provider_family": "anthropic", "session_isolation": "isolated"})
        verdict["inputs"] = [{"artifact_id": review["artifact_id"], "digest": digest(review)}]
        result = self.command("accept-result", "--handoff", review_path, "--result", self.write("verdict.json", verdict))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(review["source_implementation"]["artifact_id"], implementation["artifact_id"])
        self.assertEqual(implementation["source_spec"]["artifact_id"], spec["artifact_id"])

    def test_same_provider_reviewer_fails_required_independence(self):
        review = load("review-handoff.json")
        verdict = load("agent-result.json")
        verdict["role"] = "reviewer"
        verdict["producer"].update({"bridge": "AskCodex", "provider_family": review["required_independence"]["provider_family"], "session_isolation": "isolated"})
        verdict["inputs"] = [{"artifact_id": review["artifact_id"], "digest": digest(review)}]
        result = self.command("accept-result", "--handoff", self.write("review.json", review), "--result", self.write("verdict.json", verdict))
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_INDEPENDENCE_UNSATISFIED", result.stderr)


if __name__ == "__main__":
    unittest.main()

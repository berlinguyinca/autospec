import copy
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
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(payload).hexdigest()


class HandoffCliTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        (self.repo / "scripts").mkdir()
        (self.repo / "scripts" / "existing.py").write_text(
            "def existing_symbol():\n    return True\n", encoding="utf-8"
        )
        self.output = self.root / "output.json"

    def tearDown(self):
        self.temp.cleanup()

    def write(self, name, value):
        path = self.root / name
        path.write_text(json.dumps(value), encoding="utf-8")
        return path

    def run_cli(self, *args):
        return subprocess.run(
            ["python3", str(CLI), *map(str, args)],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def planning_results(self):
        candidate = load("spec.json")
        candidate["status"] = "proposal"
        candidate["artifact_id"] = "sha256:" + "0" * 64
        candidate["affected_surfaces"] = [
            {"path": "scripts/existing.py", "state": "existing", "symbols": ["existing_symbol"]},
            {"path": "scripts/new.py", "state": "proposed", "symbols": ["new_symbol"]},
        ]
        proposal = load("agent-result.json")
        proposal["artifact_id"] = "sha256:" + "a" * 64
        proposal["role"] = "intent_planner"
        proposal["producer"]["bridge"] = "AskClaude"
        proposal["producer"]["provider_family"] = "anthropic"
        proposal["proposed_artifact"] = candidate
        critique = load("agent-result.json")
        critique["artifact_id"] = "sha256:" + "c" * 64
        critique["role"] = "repository_critic"
        critique["producer"]["bridge"] = "AskCodex"
        critique["producer"]["provider_family"] = "openai"
        critique["proposed_artifact"] = None
        return proposal, critique

    def reconcile(self, proposal, critique):
        return self.run_cli(
            "reconcile-spec",
            "--proposal", self.write("proposal.json", proposal),
            "--critique", self.write("critique.json", critique),
            "--repo", self.repo,
            "--output", self.output,
        )

    def test_reconcile_approves_grounded_spec_and_records_source_digests(self):
        proposal, critique = self.planning_results()
        result = self.reconcile(proposal, critique)
        self.assertEqual(result.returncode, 0, result.stderr)
        artifact = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertEqual(artifact["status"], "approved")
        self.assertEqual(
            artifact["sources"],
            [
                {"artifact_id": proposal["artifact_id"], "digest": digest(proposal)},
                {"artifact_id": critique["artifact_id"], "digest": digest(critique)},
            ],
        )
        self.assertEqual(proposal["proposed_artifact"]["status"], "proposal")

    def test_reconcile_rejects_missing_existing_path_and_symbol(self):
        proposal, critique = self.planning_results()
        proposal["proposed_artifact"]["affected_surfaces"][0]["path"] = "scripts/missing.py"
        result = self.reconcile(proposal, critique)
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_EVIDENCE_INSUFFICIENT", result.stderr)
        proposal, critique = self.planning_results()
        proposal["proposed_artifact"]["affected_surfaces"][0]["symbols"] = ["missing_symbol"]
        result = self.reconcile(proposal, critique)
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_EVIDENCE_INSUFFICIENT", result.stderr)

    def test_reconcile_rejects_blocking_critique(self):
        proposal, critique = self.planning_results()
        critique["findings"][0]["severity"] = "blocking"
        result = self.reconcile(proposal, critique)
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_EVIDENCE_INSUFFICIENT", result.stderr)

    def issue(self):
        return {
            "number": 901,
            "title": "Implement typed handoffs",
            "branch": "feat/pi-agent-handoffs",
            "worktree": ".",
            "claim_generation": 1,
            "allowed_read_paths": ["scripts/", "schemas/"],
            "allowed_write_paths": ["scripts/autospec-handoff.py", "tests/pi-agent-handoff/"],
            "selected_acceptance_criteria": ["AC-1"],
            "interfaces": ["scripts/autospec-handoff.py:main"],
            "route": {"harness": "pi", "model": "openai-codex/gpt-5.5", "reasoning_effort": "medium"}
        }

    def test_implementation_rejects_unapproved_spec_and_empty_write_scope(self):
        spec = load("spec.json")
        spec["status"] = "needs_revision"
        spec["material_questions"] = ["Which route?"]
        result = self.run_cli("implementation", "--spec", self.write("spec.json", spec), "--issue", self.write("issue.json", self.issue()), "--output", self.output)
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_UNRESOLVED_MATERIAL_QUESTION", result.stderr)
        spec = load("spec.json")
        issue = self.issue()
        issue["allowed_write_paths"] = []
        result = self.run_cli("implementation", "--spec", self.write("spec.json", spec), "--issue", self.write("issue.json", issue), "--output", self.output)
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_SCOPE_INVALID", result.stderr)

    def test_implementation_derives_bounded_handoff(self):
        spec = load("spec.json")
        issue = self.issue()
        result = self.run_cli("implementation", "--spec", self.write("spec.json", spec), "--issue", self.write("issue.json", issue), "--output", self.output)
        self.assertEqual(result.returncode, 0, result.stderr)
        handoff = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertEqual(handoff["source_spec"]["digest"], digest(spec))
        self.assertEqual(handoff["allowed_write_paths"], issue["allowed_write_paths"])
        self.assertEqual([item["id"] for item in handoff["acceptance_criteria"]], ["AC-1"])

    def test_review_rejects_lineage_and_scope_mismatch(self):
        implementation = load("implementation-handoff.json")
        closeout = {
            "source_implementation": {"artifact_id": "sha256:" + "9" * 64, "digest": digest(implementation)},
            "changed_paths": ["scripts/autospec-handoff.py"],
            "claims": load("review-handoff.json")["closeout"]["claims"],
            "checks": load("review-handoff.json")["checks"]
        }
        result = self.run_cli("review", "--implementation", self.write("implementation.json", implementation), "--closeout", self.write("closeout.json", closeout), "--base", "1" * 40, "--head", "2" * 40, "--output", self.output)
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_LINEAGE_MISMATCH", result.stderr)
        closeout["source_implementation"] = {"artifact_id": implementation["artifact_id"], "digest": digest(implementation)}
        closeout["changed_paths"] = ["outside/scope.py"]
        result = self.run_cli("review", "--implementation", self.write("implementation.json", implementation), "--closeout", self.write("closeout.json", closeout), "--base", "1" * 40, "--head", "2" * 40, "--output", self.output)
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_SCOPE_INVALID", result.stderr)

    def test_validate_rejects_traversal_path(self):
        implementation = load("implementation-handoff.json")
        implementation["allowed_write_paths"] = ["../escape.py"]
        result = self.run_cli("validate", "--kind", "implementation", "--input", self.write("bad.json", implementation))
        self.assertEqual(result.returncode, 3)
        self.assertIn("HANDOFF_SCHEMA_INVALID", result.stderr)


if __name__ == "__main__":
    unittest.main()

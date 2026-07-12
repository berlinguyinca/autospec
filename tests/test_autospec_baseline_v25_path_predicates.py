import importlib.util
from types import SimpleNamespace
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT / "scripts" / "autospec-baseline-v25.py"


def load_baseline_module():
    spec = importlib.util.spec_from_file_location("autospec_baseline_v25", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def test_documentation_coverage_uses_path_tokens_not_substrings(tmp_path):
    module = load_baseline_module()
    docs = tmp_path / "docs"
    docs.mkdir()
    (docs / "prerunbook.md").write_text("# Not a runbook\n", encoding="utf-8")
    (docs / "insecurity.md").write_text("# Not security docs\n", encoding="utf-8")
    (docs / "broad-roadmapping.md").write_text("# Not roadmap docs\n", encoding="utf-8")

    payload = module.documentation_coverage(tmp_path)

    assert payload["feature_docs"]["runbooks"] is False
    assert payload["feature_docs"]["security"] is False
    assert payload["feature_docs"]["roadmap"] is False


def test_test_matrix_uses_filename_tokens_not_substrings(tmp_path):
    module = load_baseline_module()
    tests = tmp_path / "tests"
    tests.mkdir()
    (tests / "autospec-v250.bats").write_text("#!/usr/bin/env bats\n", encoding="utf-8")
    (tests / "nonregressionish.bats").write_text("#!/usr/bin/env bats\n", encoding="utf-8")

    payload = module.test_matrix(tmp_path)

    assert payload["subsystems"]["smoke"] == []
    assert payload["subsystems"]["regression"] == []


def test_generic_artifact_build_uses_registry_and_preserves_v40_v54_artifacts(tmp_path):
    module = load_baseline_module()

    assert set(module.GENERIC_ARTIFACT_BUILDERS) == set(range(40, 61))

    module.generic_artifact_build(tmp_path, 40)
    v40_root = tmp_path / ".autospec" / "autonomy" / "v40" / "autonomy-v40-ci-local-fix-simulation"
    assert (v40_root / "local-fix-simulation.json").exists()
    assert (v40_root / "disposable-fix" / "src" / "ci_fix_marker.txt").read_text(encoding="utf-8") == "autospec v40 local CI fix simulation\n"
    assert (v40_root / "update-plan.md").read_text(encoding="utf-8").startswith("# V40 Update Plan")

    module.generic_artifact_build(tmp_path, 54)
    v54_root = tmp_path / ".autospec" / "autonomy" / "v54" / "autonomy-v54-portfolio-planning"
    assert (v54_root / "portfolio-inventory.json").exists()
    assert (v54_root / "candidate-ranking.md").read_text(encoding="utf-8").startswith("# V54 Candidate Ranking")
    assert (v54_root / "shared-rule-report.json").exists()


def test_generic_gate_preserves_forbidden_flag_blockers(tmp_path):
    module = load_baseline_module()
    (tmp_path / ".git").mkdir()
    (tmp_path / ".git" / "HEAD").write_text("ref: refs/heads/feat/generic-gate-test\n", encoding="utf-8")
    previous_status = tmp_path / ".autospec" / "reports" / "autonomy-v40-status.json"
    previous_status.parent.mkdir(parents=True)
    previous_status.write_text(
        '{"status":"ready","phase_goal_satisfied":true}\n',
        encoding="utf-8",
    )
    args = SimpleNamespace(
        allow_network=True,
        execute_real_github_write=True,
        allow_git_push=True,
        allow_github_pr=True,
        allow_merge=True,
        allow_auto_merge=True,
        allow_approval=True,
        allow_self_approval=True,
        allow_default_branch_push=True,
        allow_force_push=True,
        allow_tag_push=True,
    )

    payload = module.generic_gate(tmp_path, 41, args)

    assert payload["status"] == "blocked_forbidden_operation:network_not_allowed"
    assert payload["blockers"] == sorted(
        [
            "blocked_forbidden_operation:network_not_allowed",
            "blocked_forbidden_operation:github_write_requested",
            "blocked_forbidden_operation:merge_requested",
            "blocked_forbidden_operation:approval_requested",
            "blocked_forbidden_operation:default_branch_push_requested",
            "blocked_forbidden_operation:force_push_requested",
            "blocked_forbidden_operation:tag_push_requested",
        ]
    )
    assert payload["real_write_allowed"] is False
def test_handle_generic_command_uses_dispatch_table_for_all_generic_actions(tmp_path, monkeypatch, capsys):
    module = load_baseline_module()
    actions = (
        "contract",
        "preflight",
        "artifact-build",
        "gate",
        "audit",
        "verifier",
        "recovery",
        "status",
        "supervisor",
    )

    assert tuple(name for name, _handler in module.GENERIC_COMMAND_DISPATCHERS) == actions

    calls = []

    def recorder(name, result=None):
        def _record(root, version, *extra_args):
            calls.append((name, root, version, extra_args))
            return result

        return _record

    monkeypatch.setattr(module, "generic_contract", recorder("contract"))
    monkeypatch.setattr(module, "generic_preflight", recorder("preflight", {"blockers": []}))
    monkeypatch.setattr(module, "generic_artifact_build", recorder("artifact-build"))
    monkeypatch.setattr(module, "generic_gate", recorder("gate", {"blockers": []}))
    monkeypatch.setattr(module, "generic_audit", recorder("audit"))
    monkeypatch.setattr(module, "generic_verifier", recorder("verifier", {"blockers": []}))
    monkeypatch.setattr(module, "generic_recovery", recorder("recovery"))
    monkeypatch.setattr(module, "generic_status", recorder("status", {"status": "ready"}))
    monkeypatch.setattr(module, "generic_supervisor", recorder("supervisor", {"status": "ready"}))

    for action in actions:
        args = SimpleNamespace(command=f"v40-{action}")
        assert module.handle_generic_command(tmp_path, args) == 0

    assert [call[0] for call in calls] == list(actions)
    assert all(call[1] == tmp_path and call[2] == 40 for call in calls)
    assert calls[3][3] == (SimpleNamespace(command="v40-gate"),)
    assert calls[8][3] == (SimpleNamespace(command="v40-supervisor"),)
    assert capsys.readouterr().out == "v40 status: ready\nv40 status: ready\n"

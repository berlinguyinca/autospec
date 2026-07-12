import ast
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


def test_legacy_version_commands_are_table_dispatched():
    module = load_baseline_module()

    assert set(module.LEGACY_COMMAND_READY_STATUS) == set(range(26, 40))

    module_source = MODULE_PATH.read_text(encoding="utf-8")
    module_ast = ast.parse(module_source)
    main_source = ast.get_source_segment(module_source, module_ast.body[-2])
    assert main_source is not None
    assert "handle_legacy_command(root, args)" in main_source
    assert 'args.command == "v26-' not in main_source
    assert 'args.command == "v39-' not in main_source


def test_legacy_command_dispatch_preserves_status_return_codes(tmp_path, monkeypatch, capsys):
    module = load_baseline_module()

    def fake_v26_status(root):
        return {"status": "ready_after_human_canary"}

    def fake_v28_status(root):
        return {"status": "blocked"}

    monkeypatch.setattr(module, "v26_status", fake_v26_status)
    monkeypatch.setattr(module, "v28_status", fake_v28_status)

    ready_args = SimpleNamespace(command="v26-status")
    blocked_args = SimpleNamespace(command="v28-status")

    assert module.handle_legacy_command(tmp_path, ready_args) == 0
    assert "v26 status: ready_after_human_canary" in capsys.readouterr().out
    assert module.handle_legacy_command(tmp_path, blocked_args) == 1
    assert "v28 status: blocked" in capsys.readouterr().out

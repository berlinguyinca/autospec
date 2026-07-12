import importlib.util
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

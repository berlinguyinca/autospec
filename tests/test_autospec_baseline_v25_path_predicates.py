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

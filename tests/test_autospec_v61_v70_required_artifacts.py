import importlib.util
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT / "scripts" / "autospec-v61-v70.py"


def load_v61_v70_module():
    spec = importlib.util.spec_from_file_location("autospec_v61_v70", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def test_required_artifact_builders_are_keyed_by_version():
    module = load_v61_v70_module()

    assert module.REQUIRED_ARTIFACT_BUILDERS.keys() == set(range(62, 71))
    assert all(callable(builder) for builder in module.REQUIRED_ARTIFACT_BUILDERS.values())


def test_build_required_artifacts_dispatches_by_direct_version_lookup(tmp_path):
    module = load_v61_v70_module()
    calls = []

    def fake_builder(root, base):
        calls.append((root, base))
        module.write_text(base / "sentinel.md", "called\n")

    module.REQUIRED_ARTIFACT_BUILDERS = {62: fake_builder}

    module.build_required_artifacts(tmp_path, 62)

    base = tmp_path / ".autospec" / "multirepo" / "v62"
    assert calls == [(tmp_path, base)]
    assert (base / "sentinel.md").read_text(encoding="utf-8") == "called\n"
    assert json.loads((base / "negative-proof.json").read_text(encoding="utf-8"))["status"] == "pass"
    assert json.loads((base / "artifact-index.json").read_text(encoding="utf-8"))["required"] == module.PHASES[62]["required"]


def test_build_required_artifacts_preserves_common_outputs_without_version_builder(tmp_path):
    module = load_v61_v70_module()

    module.REQUIRED_ARTIFACT_BUILDERS = {}
    module.build_required_artifacts(tmp_path, 62)

    base = tmp_path / ".autospec" / "multirepo" / "v62"
    assert sorted(path.name for path in base.iterdir()) == ["artifact-index.json", "negative-proof.json"]

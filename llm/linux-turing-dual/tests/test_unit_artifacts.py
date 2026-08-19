"""The fetch plan that makes this repository a rebuild recipe.

A truncated model is worse than a missing one: it loads, it answers, and it is
subtly wrong. So every entry must carry an exact byte count and a revision rather
than a branch -- the 27B repository was modified on the same day these weights
were first fetched.
"""
import importlib.util
import pathlib

HERE = pathlib.Path(__file__).resolve().parents[1]
SRC = HERE / "scripts" / "artifacts.py"
YAML = HERE / "config" / "model-artifacts.yaml"


def load():
    spec = importlib.util.spec_from_file_location("artifacts", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def plan():
    return load().artifact_fetch_plan(YAML.read_text())


def test_plan_covers_the_served_weights():
    files = {e["file"] for e in plan()}
    assert "Qwen3.8-27B-UD-Q4_K_M.gguf" in files
    assert "Qwen3.5-9B-Q4_K_M.gguf" in files


def test_every_entry_has_a_revision_not_a_branch():
    for e in plan():
        assert len(e["revision"]) >= 12, e
        assert e["revision"] not in ("main", "master"), e


def test_every_entry_has_a_positive_byte_count():
    for e in plan():
        assert isinstance(e["size_bytes"], int) and e["size_bytes"] > 0, e


def test_no_two_entries_write_the_same_filename():
    """Two projectors ship as mmproj-F16.gguf in different repositories.

    Without distinct local names the second download silently overwrites the
    first and a model loads the wrong projector.
    """
    files = [e["file"] for e in plan()]
    assert len(files) == len(set(files)), "two artifacts would overwrite each other"


def test_every_entry_has_a_repository():
    for e in plan():
        assert "/" in e["repository"], e


def test_empty_yaml_is_an_empty_plan_not_an_exception():
    assert load().artifact_fetch_plan("") == []


def test_yaml_without_artifacts_key_is_empty():
    assert load().artifact_fetch_plan("rejected: []\n") == []

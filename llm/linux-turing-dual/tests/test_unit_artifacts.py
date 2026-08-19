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


def test_plan_covers_every_served_artifact():
    """Assert the INVARIANT, not the filenames.

    An earlier version pinned specific quant filenames and broke the moment the 9B
    moved to a Dynamic quant -- a test that fails on an intended change teaches
    people to edit tests rather than to read them. What must hold is that every
    artifact declared in the yaml appears in the fetch plan, whatever it is called.
    """
    import yaml as _yaml
    doc = _yaml.safe_load(YAML.read_text())
    declared = {a["file"] for a in doc["artifacts"] if a.get("file")}
    declared |= {a["projector"]["file"] for a in doc["artifacts"]
                 if isinstance(a.get("projector"), dict)}
    assert {e["file"] for e in plan()} == declared


def test_plan_includes_weights_for_every_served_id():
    import yaml as _yaml
    doc = _yaml.safe_load(YAML.read_text())
    served = [a["served_as"] for a in doc["artifacts"] if a.get("served_as")]
    assert len(served) == len(set(served)), "two artifacts claim the same served_as"
    assert len(plan()) >= len(served), "an artifact has no file to fetch"


def test_every_entry_has_a_revision_not_a_branch():
    for e in plan():
        assert len(e["revision"]) >= 12, e
        assert e["revision"] not in ("main", "master"), e


def test_every_entry_has_a_positive_byte_count():
    for e in plan():
        assert isinstance(e["size_bytes"], int) and e["size_bytes"] > 0, e


def test_no_two_entries_write_the_same_local_file():
    """Two projectors ship as mmproj-F16.gguf in different repositories.

    Without distinct LOCAL names the second download silently overwrites the first
    and a model loads the wrong projector -- which still answers, plausibly.
    """
    dests = [e["dest"] for e in plan()]
    assert len(dests) == len(set(dests)), "two artifacts would overwrite each other"


def test_remote_and_local_names_may_differ():
    """The collision is resolved by local_file, so a remote name may repeat."""
    for e in plan():
        assert e["dest"], e
        assert e["file"], e


def test_every_entry_has_a_local_destination():
    for e in plan():
        assert "/" not in e["dest"], "dest must be a bare filename: %r" % e


def test_every_entry_has_a_repository():
    for e in plan():
        assert "/" in e["repository"], e


def test_empty_yaml_is_an_empty_plan_not_an_exception():
    assert load().artifact_fetch_plan("") == []


def test_yaml_without_artifacts_key_is_empty():
    assert load().artifact_fetch_plan("rejected: []\n") == []

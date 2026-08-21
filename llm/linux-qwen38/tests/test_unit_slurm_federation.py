from __future__ import annotations

import configparser
import json
import subprocess
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
SLURM = HERE.parent / "slurm"
GEN = SLURM / "gen-preset.py"
sys.path.insert(0, str(SLURM))

import federation_contract as fc


def _write_config(
    tmp_path: Path,
    *,
    mode: int = 0o600,
    node: str = "gateway.example.test",
    server_id: str = "cluster-alice-01",
    target: str = "http://127.0.0.1:8080",
    credential: str = "qts_example_secret",
) -> Path:
    path = tmp_path / "agent-config.json"
    path.write_text(
        json.dumps(
            {
                "node": node,
                "server_id": server_id,
                "target": target,
                "credential": credential,
            }
        )
    )
    path.chmod(mode)
    return path


def _run_federated(vram_mib: int) -> configparser.ConfigParser:
    res = subprocess.run(
        [
            sys.executable,
            str(GEN),
            "--vram-mib",
            str(vram_mib),
            "--models-dir",
            "/m",
            "--federated",
        ],
        capture_output=True,
        text=True,
        check=False,
        timeout=120,
    )
    if res.returncode != 0:
        raise AssertionError(res.stderr.strip())
    parser = configparser.ConfigParser(interpolation=None)
    parser.optionxform = str
    parser.read_string("[__top__]\n" + res.stdout)
    return parser


def test_primary_model_contract_is_exactly_the_established_seven():
    assert fc.PRIMARY_MODELS == {
        "qwen3.8-27b",
        "qwen3.8-27b-100k",
        "qwen3.8-27b-vision",
        "qwen3.8-27b-uncensored",
        "qwen3.8-27b-uncensored-vision",
        "qwen3.5-9b",
        "qwen3.5-9b-vision",
    }


@pytest.mark.parametrize(
    ("payload", "expected"),
    [
        (
            {"data": [{"id": "qwen3.8-27b"}, {"id": "qwen3.8-27b"}]},
            {"qwen3.8-27b"},
        ),
        (
            json.dumps({"data": [{"id": "qwen3.8-27b-vision"}]}),
            {"qwen3.8-27b-vision"},
        ),
        (
            json.dumps({"data": [{"id": "qwen3.8-27b-uncensored"}]}).encode(),
            {"qwen3.8-27b-uncensored"},
        ),
    ],
)
def test_model_ids_accepts_dict_string_and_bytes_payloads(payload, expected):
    assert fc.model_ids(payload) == expected


def test_model_ids_deduplicates_duplicate_ids():
    payload = {
        "data": [
            {"id": "qwen3.8-27b"},
            {"id": "qwen3.8-27b"},
            {"id": "qwen3.8-27b-vision"},
        ]
    }
    assert fc.model_ids(payload) == {"qwen3.8-27b", "qwen3.8-27b-vision"}


@pytest.mark.parametrize(
    "payload",
    [
        "{not json",
        {"data": {"id": "qwen3.8-27b"}},
        {"data": [{"id": 7}]},
        {"data": ["qwen3.8-27b"]},
    ],
)
def test_model_ids_rejects_malformed_shapes_and_nonstring_ids(payload):
    with pytest.raises(ValueError) as excinfo:
        fc.model_ids(payload)
    assert str(excinfo.value)


def test_nonempty_subset_is_accepted():
    assert fc.validate_scheduler_subset({"qwen3.8-27b"}) == {"qwen3.8-27b"}


@pytest.mark.parametrize("ids", [set(), {"qwen3.8-27b-40k"}, {"other-model"}])
def test_empty_alias_or_unknown_model_is_rejected(ids):
    with pytest.raises(ValueError) as excinfo:
        fc.validate_scheduler_subset(ids)
    assert str(excinfo.value)


def test_agent_config_must_be_private_and_loopback_targeted(tmp_path: Path):
    path = _write_config(tmp_path, mode=0o644, target="http://0.0.0.0:8080")
    with pytest.raises(ValueError) as excinfo:
        fc.load_agent_config(path, 8080)
    assert "0600" in str(excinfo.value)


def test_agent_config_rejects_invalid_server_id(tmp_path: Path):
    path = _write_config(tmp_path, server_id="Cluster_Alice_01")
    with pytest.raises(ValueError) as excinfo:
        fc.load_agent_config(path, 8080)
    assert "server_id" in str(excinfo.value)


@pytest.mark.parametrize(
    ("field", "value"),
    [("node", ""), ("credential", ""), ("target", "http://localhost:8080")],
)
def test_agent_config_rejects_empty_required_values_and_non_exact_target(
    tmp_path: Path, field: str, value: str
):
    kwargs = {field: value}
    path = _write_config(tmp_path, **kwargs)
    with pytest.raises(ValueError) as excinfo:
        fc.load_agent_config(path, 8080)
    assert str(excinfo.value)


def test_agent_config_returns_redacted_metadata_only(tmp_path: Path):
    path = _write_config(tmp_path)
    cfg = fc.load_agent_config(path, 8080)
    assert cfg == {
        "node": "gateway.example.test",
        "server_id": "cluster-alice-01",
        "target": "http://127.0.0.1:8080",
        "has_credential": True,
    }


@pytest.mark.parametrize("vram_mib", [97887, 81920, 46068, 40960])
def test_federated_preset_only_advertises_primary_model_ids_without_aliases(vram_mib: int):
    parser = _run_federated(vram_mib)
    section_ids = {name for name in parser.sections() if name not in {"*", "__top__"}}
    assert section_ids
    assert fc.validate_scheduler_subset(section_ids) == section_ids
    for section in section_ids:
        assert not parser.has_option(section, "alias")

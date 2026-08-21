from __future__ import annotations

import json
import re
import stat
from pathlib import Path

PRIMARY_MODELS = frozenset(
    {
        "qwen3.8-27b",
        "qwen3.8-27b-100k",
        "qwen3.8-27b-vision",
        "qwen3.8-27b-uncensored",
        "qwen3.8-27b-uncensored-vision",
        "qwen3.5-9b",
        "qwen3.5-9b-vision",
    }
)

_SERVER_ID_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,30}$")


def _models_doc(payload: bytes | str | dict) -> dict:
    if isinstance(payload, dict):
        doc = payload
    elif isinstance(payload, bytes):
        try:
            doc = json.loads(payload.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise ValueError("model payload must be valid UTF-8 JSON") from exc
    elif isinstance(payload, str):
        try:
            doc = json.loads(payload)
        except json.JSONDecodeError as exc:
            raise ValueError("model payload must be valid JSON") from exc
    else:
        raise ValueError("model payload must be bytes, str or dict")
    if not isinstance(doc, dict):
        raise ValueError("model payload must decode to an object")
    return doc


def model_ids(payload: bytes | str | dict) -> set[str]:
    doc = _models_doc(payload)
    data = doc.get("data")
    if not isinstance(data, list):
        raise ValueError("model payload must contain a list-valued data field")
    ids: set[str] = set()
    for entry in data:
        if not isinstance(entry, dict):
            raise ValueError("each model entry must be an object")
        mid = entry.get("id")
        if not isinstance(mid, str) or not mid:
            raise ValueError("each model entry must carry a non-empty string id")
        ids.add(mid)
    return ids


def validate_scheduler_subset(ids: set[str]) -> set[str]:
    if not ids:
        raise ValueError("scheduler workers must advertise a non-empty model subset")
    unknown = sorted(set(ids) - PRIMARY_MODELS)
    if unknown:
        raise ValueError(
            "scheduler workers may advertise only established primary ids: "
            + ", ".join(unknown)
        )
    return set(ids)


def load_agent_config(path: Path, expected_port: int) -> dict:
    st = path.stat()
    if not stat.S_ISREG(st.st_mode):
        raise ValueError("agent config must be a regular file")
    if stat.S_IMODE(st.st_mode) != 0o600:
        raise ValueError("agent config must have mode 0600")

    try:
        doc = json.loads(path.read_text())
    except json.JSONDecodeError as exc:
        raise ValueError("agent config must be valid JSON") from exc
    if not isinstance(doc, dict):
        raise ValueError("agent config must be a JSON object")

    node = doc.get("node")
    if not isinstance(node, str) or not node.strip():
        raise ValueError("agent config must include a non-empty node")

    server_id = doc.get("server_id")
    if not isinstance(server_id, str) or not _SERVER_ID_RE.match(server_id):
        raise ValueError(
            "agent config server_id must be lowercase letters, digits and dashes"
        )

    target = doc.get("target")
    expected_target = f"http://127.0.0.1:{expected_port}"
    if target != expected_target:
        raise ValueError(f"agent target must be exactly {expected_target}")

    credential = doc.get("credential")
    if not isinstance(credential, str) or not credential.strip():
        raise ValueError("agent config must include a non-empty credential")

    return {
        "node": node,
        "server_id": server_id,
        "target": target,
        "has_credential": True,
    }

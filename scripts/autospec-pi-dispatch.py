#!/usr/bin/env python3
"""Execute one routed AutoSpec dispatch through Pi JSON mode."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


def _fail(code: int, message: str) -> int:
    print(message, file=sys.stderr)
    return code


def _load_envelope(path: Path) -> dict[str, Any]:
    try:
        envelope = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"invalid dispatch envelope: {exc}") from exc
    if not isinstance(envelope, dict):
        raise ValueError("invalid dispatch envelope: expected object")
    return envelope


def _validate(envelope: dict[str, Any]) -> None:
    if envelope.get("version") != 1:
        raise ValueError("ROUTING_ADAPTER_UNSUPPORTED: envelope version must be 1")
    if envelope.get("harness", {}).get("id") != "pi":
        raise ValueError("ROUTING_ADAPTER_UNSUPPORTED: envelope harness must be pi")
    if envelope.get("harness", {}).get("transport") != "json":
        raise ValueError("ROUTING_ADAPTER_UNSUPPORTED: Pi version 1 requires json transport")
    if envelope.get("inference", {}).get("protocol") != "openai-compatible":
        raise ValueError("ROUTING_ADAPTER_UNSUPPORTED: Pi requires openai-compatible inference")


def _provider_config(envelope: dict[str, Any]) -> dict[str, Any]:
    inference = envelope["inference"]
    harness = envelope["harness"]
    api_key_env = harness.get("api_key_env", "INFERWEAVE_API_KEY")
    return {
        "providers": {
            "autospec-inferweave": {
                "baseUrl": inference["endpoint"],
                "api": "openai-completions",
                "apiKey": api_key_env,
                "authHeader": True,
                "compat": {
                    "supportsDeveloperRole": False,
                    "supportsReasoningEffort": False,
                },
                "models": [
                    {
                        "id": inference["model"],
                        "name": inference["model"],
                        "reasoning": True,
                        "input": list(inference["modalities"]),
                        "contextWindow": inference["context_window"],
                        "maxTokens": inference["reserve_output_tokens"],
                        "cost": {
                            "input": 0,
                            "output": 0,
                            "cacheRead": 0,
                            "cacheWrite": 0,
                        },
                    }
                ],
            }
        }
    }


def _text(content: Any) -> str:
    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return ""
    return "".join(
        item.get("text", "")
        for item in content
        if isinstance(item, dict) and item.get("type") == "text"
    )


def _normalize(lines: list[str], child_status: int) -> dict[str, Any]:
    events = []
    for line in lines:
        if not line.strip():
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError as exc:
            raise ValueError(f"malformed Pi JSONL: {exc}") from exc
        if not isinstance(event, dict):
            raise ValueError("malformed Pi JSONL: event must be an object")
        events.append(event)
    message = ""
    usage: dict[str, Any] = {}
    for event in events:
        if event.get("type") == "message_end":
            candidate = event.get("message", {})
            if candidate.get("role") == "assistant":
                message = _text(candidate.get("content"))
                usage = candidate.get("usage") or event.get("usage") or {}
        elif event.get("type") == "result" and isinstance(event.get("message"), str):
            message = event["message"]
            usage = event.get("usage") or usage
    if not message and child_status == 0:
        raise ValueError("malformed Pi JSONL: no final assistant message")
    return {
        "message": message,
        "usage": {
            "input_tokens": int(usage.get("input", usage.get("input_tokens", 0)) or 0),
            "output_tokens": int(usage.get("output", usage.get("output_tokens", 0)) or 0),
            "cached_tokens": int(usage.get("cacheRead", usage.get("cached_tokens", 0)) or 0),
            "cache_write_tokens": int(usage.get("cacheWrite", usage.get("cache_write_tokens", 0)) or 0),
        },
        "child_exit_status": child_status,
    }


def run_pi_dispatch(envelope: dict[str, Any], prompt_path: Path, env: dict[str, str]) -> tuple[dict[str, Any], int]:
    _validate(envelope)
    pi = shutil.which("pi", path=env.get("PATH"))
    if not pi:
        raise FileNotFoundError("Pi executable not found")
    if not prompt_path.is_file():
        raise ValueError(f"prompt file not found: {prompt_path}")
    with tempfile.TemporaryDirectory(prefix="autospec-pi-") as temp:
        pi_dir = Path(temp)
        (pi_dir / "models.json").write_text(
            json.dumps(_provider_config(envelope), sort_keys=True, separators=(",", ":")),
            encoding="utf-8",
        )
        child_env = dict(env)
        child_env["PI_CODING_AGENT_DIR"] = str(pi_dir)
        command = [
            pi,
            "--provider",
            "autospec-inferweave",
            "--model",
            envelope["inference"]["model"],
            "--mode",
            "json",
            "--print",
            "--no-session",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--tools",
            "read,grep,find,edit,write,bash",
            f"@{prompt_path.resolve()}",
        ]
        completed = subprocess.run(
            command,
            env=child_env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    if completed.stderr:
        print(completed.stderr, file=sys.stderr, end="")
    return _normalize(completed.stdout.splitlines(), completed.returncode), completed.returncode


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--envelope", required=True)
    parser.add_argument("--prompt-file", required=True)
    args = parser.parse_args(argv)
    try:
        envelope = _load_envelope(Path(args.envelope))
        payload, status = run_pi_dispatch(envelope, Path(args.prompt_file), dict(os.environ))
    except FileNotFoundError as exc:
        return _fail(2, f"ROUTING_HARNESS_UNAVAILABLE: {exc}")
    except ValueError as exc:
        message = str(exc)
        code = 3 if message.startswith("ROUTING_ADAPTER_UNSUPPORTED") else 1
        return _fail(code, message)
    print(json.dumps(payload, sort_keys=True, separators=(",", ":")))
    return status


if __name__ == "__main__":
    raise SystemExit(main())

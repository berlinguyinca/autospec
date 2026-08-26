#!/usr/bin/env python3
"""Run one allowlisted Pi bridge role and emit a validated handoff result."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
HANDOFF_CLI = ROOT / "scripts" / "autospec-handoff.py"
ROLES = {"intent_planner", "repository_critic", "implementation_advisor", "reviewer"}
READ_ROLES = ROLES


class BridgeError(Exception):
    def __init__(self, category: str, detail: str):
        super().__init__(detail)
        self.category = category


def canonical(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def digest(value: Any) -> str:
    return "sha256:" + hashlib.sha256(canonical(value).encode("utf-8")).hexdigest()


def require_keys(value: dict[str, Any], allowed: set[str], required: set[str], name: str) -> None:
    unknown = set(value) - allowed
    missing = required - set(value)
    if unknown or missing:
        details = []
        if unknown:
            details.append(f"unknown keys: {', '.join(sorted(unknown))}")
        if missing:
            details.append(f"missing keys: {', '.join(sorted(missing))}")
        raise BridgeError("HANDOFF_SCHEMA_INVALID", f"{name} {'; '.join(details)}")


def load_config(path: Path) -> dict[str, Any]:
    try:
        import yaml
    except ImportError as exc:
        raise BridgeError("HANDOFF_SCHEMA_INVALID", "PyYAML is required for Pi bridge configuration") from exc
    try:
        config = yaml.safe_load(path.read_text(encoding="utf-8"))
    except (OSError, yaml.YAMLError) as exc:
        raise BridgeError("HANDOFF_SCHEMA_INVALID", f"invalid Pi bridge configuration: {exc}") from exc
    if not isinstance(config, dict):
        raise BridgeError("HANDOFF_SCHEMA_INVALID", "Pi bridge configuration must be an object")
    require_keys(config, {"version", "enabled", "orchestrator", "bridges", "policy"}, {"version", "enabled", "orchestrator", "bridges", "policy"}, "configuration")
    if config["version"] != 1:
        raise BridgeError("HANDOFF_SCHEMA_INVALID", "configuration version must be 1")
    if not isinstance(config["enabled"], bool):
        raise BridgeError("HANDOFF_SCHEMA_INVALID", "enabled must be boolean")
    if not config["enabled"]:
        raise BridgeError("HANDOFF_BRIDGE_DISABLED", "Pi bridge configuration is disabled")
    orchestrator = config["orchestrator"]
    if not isinstance(orchestrator, dict):
        raise BridgeError("HANDOFF_SCHEMA_INVALID", "orchestrator must be an object")
    require_keys(orchestrator, {"provider", "model"}, {"provider", "model"}, "orchestrator")
    policy = config["policy"]
    if not isinstance(policy, dict):
        raise BridgeError("HANDOFF_SCHEMA_INVALID", "policy must be an object")
    require_keys(policy, {"max_parallel", "recursive_delegation", "require_isolated_planning_sessions"}, {"max_parallel", "recursive_delegation", "require_isolated_planning_sessions"}, "policy")
    if policy["max_parallel"] != 2 or policy["recursive_delegation"] is not False or policy["require_isolated_planning_sessions"] is not True:
        raise BridgeError("HANDOFF_SCHEMA_INVALID", "version 1 requires max_parallel=2, recursive_delegation=false, and isolated planning sessions")
    bridges = config["bridges"]
    if not isinstance(bridges, dict) or not bridges:
        raise BridgeError("HANDOFF_SCHEMA_INVALID", "bridges must be a non-empty object")
    if set(bridges) - ROLES:
        raise BridgeError("HANDOFF_SCHEMA_INVALID", "bridges contains an unsupported role")
    for role, bridge in bridges.items():
        if not isinstance(bridge, dict):
            raise BridgeError("HANDOFF_SCHEMA_INVALID", f"bridges.{role} must be an object")
        require_keys(bridge, {"package", "tool", "provider_family", "model", "reasoning_effort"}, {"package", "tool", "provider_family", "model", "reasoning_effort"}, f"bridges.{role}")
        if not re.fullmatch(r"npm:(?:@[^/\s]+/)?[^@/\s]+@[0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?", bridge["package"]):
            raise BridgeError("HANDOFF_BRIDGE_UNAVAILABLE", f"bridges.{role}.package must pin an exact npm version")
        if bridge["tool"] not in {"AskClaude", "AskCodex"}:
            raise BridgeError("HANDOFF_TOOL_UNAVAILABLE", f"unsupported bridge tool: {bridge['tool']}")
    return config


def load_input(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise BridgeError("HANDOFF_SCHEMA_INVALID", f"invalid input artifact: {exc}") from exc
    if not isinstance(value, dict) or not isinstance(value.get("artifact_id"), str):
        raise BridgeError("HANDOFF_SCHEMA_INVALID", "input artifact must contain artifact_id")
    return value


def installed_packages(pi: str, env: dict[str, str]) -> dict[str, Path]:
    completed = subprocess.run([pi, "list"], env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if completed.returncode != 0:
        raise BridgeError("HANDOFF_BRIDGE_UNAVAILABLE", completed.stderr.strip() or "pi list failed")
    packages: dict[str, Path] = {}
    source: str | None = None
    for line in completed.stdout.splitlines():
        if line.startswith("  ") and not line.startswith("    "):
            source = line.strip()
        elif source and line.startswith("    "):
            path = Path(line.strip())
            if path.is_absolute():
                packages[source] = path
            source = None
    return packages


def extension_paths(source: str, package_root: Path) -> list[Path]:
    package_spec = source.removeprefix("npm:")
    name, version = package_spec.rsplit("@", 1)
    manifest_path = package_root / "package.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise BridgeError("HANDOFF_BRIDGE_UNAVAILABLE", f"invalid installed package manifest: {exc}") from exc
    if manifest.get("name") != name or manifest.get("version") != version:
        raise BridgeError("HANDOFF_BRIDGE_UNAVAILABLE", f"installed package does not match {source}")
    entries = manifest.get("pi", {}).get("extensions")
    if not isinstance(entries, list) or not entries:
        raise BridgeError("HANDOFF_BRIDGE_UNAVAILABLE", f"installed package has no Pi extensions: {source}")
    resolved = []
    root = package_root.resolve()
    for entry in entries:
        if not isinstance(entry, str):
            raise BridgeError("HANDOFF_BRIDGE_UNAVAILABLE", f"invalid extension entry in {source}")
        path = (root / entry).resolve()
        if root not in path.parents or not path.is_file():
            raise BridgeError("HANDOFF_BRIDGE_UNAVAILABLE", f"unsafe or missing extension entry in {source}")
        resolved.append(path)
    return resolved


def tool_request(role: str, bridge: dict[str, Any], artifact: dict[str, Any], repo: Path) -> dict[str, Any]:
    if role == "intent_planner":
        output_contract = {
            "proposed_artifact": "autospec-spec-v1 status=proposal",
            "sources": [],
            "planning_evidence": [],
            "findings": "planning decisions, assumptions, and material questions",
        }
    elif role == "repository_critic":
        output_contract = {
            "proposed_artifact": None,
            "findings": "verify repository paths and symbols; mark contradictions blocking",
        }
    elif role == "reviewer":
        output_contract = {
            "proposed_artifact": None,
            "findings": "verify every acceptance criterion and cited proof artifact",
        }
    else:
        output_contract = {
            "proposed_artifact": None,
            "findings": "bounded implementation diagnosis only",
        }
    prompt = canonical({
        "instruction": "Return exactly one autospec-agent-handoff-result-v1 JSON object. Do not wrap it in Markdown.",
        "role": role,
        "input_artifact": artifact,
        "output_contract": output_contract,
        "repository": str(repo),
        "recursive_delegation": False,
    })
    if bridge["tool"] == "AskClaude":
        arguments = {"prompt": prompt, "mode": "read", "model": bridge["model"], "thinking": bridge["reasoning_effort"], "isolated": True}
    else:
        arguments = {"prompt": prompt, "model": bridge["model"], "reasoningEffort": bridge["reasoning_effort"], "sandbox": "read-only", "cwd": str(repo)}
    return {
        "instruction": f"Invoke the {bridge['tool']} tool exactly once with the supplied arguments, then return only its final JSON result.",
        "tool": bridge["tool"],
        "arguments": arguments,
    }


def final_text(lines: list[str], status: int) -> tuple[str, dict[str, int]]:
    message = ""
    usage: dict[str, int] = {}
    for line in lines:
        if not line.strip():
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError as exc:
            raise BridgeError("HANDOFF_AGENT_OUTPUT_INVALID", f"malformed Pi JSONL: {exc}") from exc
        if event.get("type") == "message_end" and event.get("message", {}).get("role") == "assistant":
            content = event["message"].get("content", [])
            message = "".join(item.get("text", "") for item in content if isinstance(item, dict) and item.get("type") == "text")
            usage = event["message"].get("usage", {})
        elif event.get("type") == "result" and isinstance(event.get("message"), str):
            message = event["message"]
            usage = event.get("usage", usage)
    if not message and status == 0:
        raise BridgeError("HANDOFF_AGENT_OUTPUT_INVALID", "Pi emitted no final assistant message")
    return message, usage


def validate_result(value: dict[str, Any], role: str, bridge: dict[str, Any], artifact: dict[str, Any], env: dict[str, str]) -> None:
    if value.get("role") != role or value.get("producer", {}).get("bridge") != bridge["tool"] or value.get("producer", {}).get("provider_family") != bridge["provider_family"]:
        raise BridgeError("HANDOFF_LINEAGE_MISMATCH", "agent result provenance does not match the requested bridge role")
    expected = [{"artifact_id": artifact["artifact_id"], "digest": digest(artifact)}]
    if value.get("inputs") != expected or value.get("producer", {}).get("session_isolation") != "isolated":
        raise BridgeError("HANDOFF_LINEAGE_MISMATCH", "agent result input lineage or isolation does not match")
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
        path = Path(handle.name)
        handle.write(canonical(value))
    try:
        completed = subprocess.run([sys.executable, str(HANDOFF_CLI), "validate", "--kind", "result", "--input", str(path)], env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    finally:
        path.unlink(missing_ok=True)
    if completed.returncode != 0:
        raise BridgeError("HANDOFF_AGENT_OUTPUT_INVALID", completed.stderr.strip())


def output_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", dir=path.parent, delete=False, encoding="utf-8") as handle:
        temp = Path(handle.name)
        handle.write(canonical(value) + "\n")
    temp.replace(path)


def run(args: argparse.Namespace, env: dict[str, str]) -> int:
    config = load_config(args.config)
    bridge = config["bridges"].get(args.role)
    if not bridge:
        raise BridgeError("HANDOFF_TOOL_UNAVAILABLE", f"no bridge configured for role: {args.role}")
    artifact = load_input(args.input)
    pi = shutil.which("pi", path=env.get("PATH"))
    if not pi:
        raise BridgeError("HANDOFF_BRIDGE_UNAVAILABLE", "Pi executable not found")
    package_root = installed_packages(pi, env).get(bridge["package"])
    if not package_root:
        raise BridgeError("HANDOFF_BRIDGE_UNAVAILABLE", f"pinned package is not installed: {bridge['package']}")
    extensions = extension_paths(bridge["package"], package_root)
    request = tool_request(args.role, bridge, artifact, args.repo.resolve())
    with tempfile.TemporaryDirectory(prefix="autospec-pi-bridge-") as temp:
        private = Path(temp)
        (private / "settings.json").write_text("{}\n", encoding="utf-8")
        prompt = private / "prompt.json"
        prompt.write_text(canonical(request), encoding="utf-8")
        child_env = dict(env)
        child_env["PI_CODING_AGENT_DIR"] = str(private)
        command = [
            pi, "--provider", config["orchestrator"]["provider"], "--model", config["orchestrator"]["model"],
            "--mode", "json", "--print", "--no-session", "--no-extensions", "--no-skills", "--no-prompt-templates",
            "--tools", "read,grep,find,ls",
        ]
        for extension in extensions:
            command.extend(["--extension", str(extension)])
        command.append(f"@{prompt}")
        completed = subprocess.run(command, cwd=args.repo, env=child_env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
        if completed.stderr:
            print(completed.stderr, file=sys.stderr, end="")
        message, _usage = final_text(completed.stdout.splitlines(), completed.returncode)
    if completed.returncode != 0:
        raise BridgeError("HANDOFF_AGENT_FAILED", f"Pi exited {completed.returncode}")
    try:
        value = json.loads(message)
    except json.JSONDecodeError as exc:
        raise BridgeError("HANDOFF_AGENT_OUTPUT_INVALID", f"final bridge result is not JSON: {exc}") from exc
    if not isinstance(value, dict):
        raise BridgeError("HANDOFF_AGENT_OUTPUT_INVALID", "final bridge result must be an object")
    validate_result(value, args.role, bridge, artifact, env)
    output_json(args.output, value)
    print(json.dumps({"artifact_id": value["artifact_id"], "output": str(args.output)}, sort_keys=True))
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--role", choices=sorted(ROLES), required=True)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repo", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        return run(args, dict(os.environ))
    except BridgeError as exc:
        print(f"{exc.category}: {exc}", file=sys.stderr)
        return 3
    except (KeyError, TypeError, ValueError) as exc:
        print(f"HANDOFF_SCHEMA_INVALID: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

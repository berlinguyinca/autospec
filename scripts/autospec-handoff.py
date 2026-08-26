#!/usr/bin/env python3
"""Validate and derive immutable AutoSpec agent handoff artifacts."""

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
from datetime import UTC, datetime
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SCHEMA_NAMES = {
    "spec": "autospec-spec-v1.schema.json",
    "implementation": "autospec-implementation-handoff-v1.schema.json",
    "review": "autospec-review-handoff-v1.schema.json",
    "result": "autospec-agent-handoff-result-v1.schema.json",
    "closeout": "autospec-implementation-closeout-v1.schema.json",
}


class HandoffError(Exception):
    def __init__(self, category: str, detail: str):
        super().__init__(detail)
        self.category = category


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")


def digest(value: Any) -> str:
    return "sha256:" + hashlib.sha256(canonical(value)).hexdigest()


def artifact_id(value: dict[str, Any]) -> str:
    unsigned = dict(value)
    unsigned.pop("artifact_id", None)
    return digest(unsigned)


def timestamp() -> str:
    return datetime.now(UTC).isoformat(timespec="seconds").replace("+00:00", "Z")


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise HandoffError("HANDOFF_SCHEMA_INVALID", f"{path}: {exc}") from exc
    if not isinstance(value, dict):
        raise HandoffError("HANDOFF_SCHEMA_INVALID", f"{path}: expected object")
    return value


def schemas_dir() -> Path:
    override = os.environ.get("AUTOSPEC_SCHEMAS_DIR")
    return Path(override) if override else ROOT / "schemas"


def validate_artifact(kind: str, value: dict[str, Any]) -> None:
    schema_name = SCHEMA_NAMES.get(kind)
    if not schema_name:
        raise HandoffError("HANDOFF_SCHEMA_INVALID", f"unknown artifact kind: {kind}")
    validator = shutil.which("ajv")
    if not validator:
        raise HandoffError("HANDOFF_SCHEMA_INVALID", "ajv is required for handoff validation")
    schema = schemas_dir() / schema_name
    if not schema.is_file():
        raise HandoffError("HANDOFF_SCHEMA_INVALID", f"missing schema: {schema}")
    with tempfile.TemporaryDirectory(prefix="autospec-handoff-validate-") as temp:
        data = Path(temp) / "artifact.json"
        data.write_bytes(canonical(value))
        result = subprocess.run(
            [validator, "validate", "--spec=draft2020", "-s", str(schema), "-d", str(data)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip().replace("\n", " ")
        raise HandoffError("HANDOFF_SCHEMA_INVALID", detail)


def safe_repo_path(root: Path, relative: str) -> Path:
    candidate = Path(relative)
    if candidate.is_absolute() or ".." in candidate.parts:
        raise HandoffError("HANDOFF_SCOPE_INVALID", relative)
    base = root.resolve()
    resolved = (base / candidate).resolve()
    if resolved != base and base not in resolved.parents:
        raise HandoffError("HANDOFF_SCOPE_INVALID", relative)
    return resolved


def output_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("wb", dir=path.parent, delete=False) as handle:
        temp = Path(handle.name)
        handle.write(canonical(value) + b"\n")
    temp.replace(path)


def producer() -> dict[str, str]:
    return {
        "agent_id": "autospec-handoff",
        "harness": "pi",
        "bridge": "native",
        "provider_family": "local",
        "model": "deterministic",
        "session_isolation": "isolated",
        "access_mode": "read",
    }


def evidence_is_grounded(root: Path, surface: dict[str, Any]) -> None:
    path = safe_repo_path(root, surface["path"])
    if surface["state"] == "proposed":
        return
    if not path.is_file():
        raise HandoffError("HANDOFF_EVIDENCE_INSUFFICIENT", f"missing existing path: {surface['path']}")
    text = path.read_text(encoding="utf-8", errors="replace")
    for symbol in surface["symbols"]:
        if not re.search(rf"(?<![A-Za-z0-9_]){re.escape(symbol)}(?![A-Za-z0-9_])", text):
            raise HandoffError("HANDOFF_EVIDENCE_INSUFFICIENT", f"missing symbol {symbol}: {surface['path']}")


def reconcile_spec(proposal: dict[str, Any], critique: dict[str, Any], repo: Path) -> dict[str, Any]:
    validate_artifact("result", proposal)
    validate_artifact("result", critique)
    if proposal["role"] != "intent_planner" or critique["role"] != "repository_critic":
        raise HandoffError("HANDOFF_LINEAGE_MISMATCH", "planning roles are not intent_planner then repository_critic")
    if proposal["producer"]["session_isolation"] != "isolated" or critique["producer"]["session_isolation"] != "isolated":
        raise HandoffError("HANDOFF_LINEAGE_MISMATCH", "planning sessions must be isolated")
    if any(item["severity"] == "blocking" for item in critique["findings"]):
        raise HandoffError("HANDOFF_EVIDENCE_INSUFFICIENT", "repository critique contains blocking findings")
    candidate = proposal.get("proposed_artifact")
    if not isinstance(candidate, dict):
        raise HandoffError("HANDOFF_SCHEMA_INVALID", "intent planner did not propose a specification")
    if candidate.get("material_questions"):
        raise HandoffError("HANDOFF_UNRESOLVED_MATERIAL_QUESTION", "proposal contains material questions")
    for surface in candidate.get("affected_surfaces", []):
        evidence_is_grounded(repo, surface)
    result = dict(candidate)
    result["status"] = "approved"
    result["created_at"] = timestamp()
    result["sources"] = [
        {"artifact_id": proposal["artifact_id"], "digest": digest(proposal)},
        {"artifact_id": critique["artifact_id"], "digest": digest(critique)},
    ]
    result["planning_evidence"] = [
        {"role": "intent_planner", "result_artifact_id": proposal["artifact_id"], "digest": digest(proposal)},
        {"role": "repository_critic", "result_artifact_id": critique["artifact_id"], "digest": digest(critique)},
    ]
    result["producer"] = producer()
    result["artifact_id"] = artifact_id(result)
    validate_artifact("spec", result)
    return result


def validate_scope_paths(paths: Any, repo: Path | None = None) -> list[str]:
    if not isinstance(paths, list) or not paths:
        raise HandoffError("HANDOFF_SCOPE_INVALID", "scope must contain at least one path")
    for path in paths:
        if not isinstance(path, str):
            raise HandoffError("HANDOFF_SCOPE_INVALID", "scope paths must be strings")
        safe_repo_path(repo or ROOT, path)
    return paths


def derive_implementation(spec: dict[str, Any], issue: dict[str, Any]) -> dict[str, Any]:
    validate_artifact("spec", spec)
    if spec["status"] != "approved" or spec["material_questions"]:
        raise HandoffError("HANDOFF_UNRESOLVED_MATERIAL_QUESTION", "source specification is not approved")
    read_paths = validate_scope_paths(issue.get("allowed_read_paths"))
    write_paths = validate_scope_paths(issue.get("allowed_write_paths"))
    selected = set(issue.get("selected_acceptance_criteria", []))
    criteria = [item for item in spec["acceptance_criteria"] if item["id"] in selected]
    if not criteria or len(criteria) != len(selected):
        raise HandoffError("HANDOFF_SCOPE_INVALID", "selected acceptance criteria are missing")
    issue_core = {key: issue[key] for key in ("number", "title", "branch", "worktree", "claim_generation")}
    result = {
        "version": 1,
        "artifact_id": "sha256:" + "0" * 64,
        "created_at": timestamp(),
        "repository": spec["repository"],
        "source_spec": {"artifact_id": spec["artifact_id"], "digest": digest(spec)},
        "producer": producer(),
        "issue": issue_core,
        "allowed_read_paths": read_paths,
        "allowed_write_paths": write_paths,
        "goal": spec["goal"],
        "acceptance_criteria": criteria,
        "constraints": spec["constraints"],
        "invariants": spec["invariants"],
        "interfaces": issue.get("interfaces", []),
        "tests_required": spec["tests_required"],
        "primary_smoke_test": spec["primary_smoke_test"],
        "limits": {"max_tool_calls": 40, "max_self_review_iterations": 3},
        "retry_findings": issue.get("retry_findings", []),
        "closeout": {"path": f".autospec/handoffs/{issue['number']}/closeout.json", "schema": "autospec-implementation-closeout-v1.schema.json"},
        "route": issue["route"],
    }
    result["artifact_id"] = artifact_id(result)
    validate_artifact("implementation", result)
    return result


def in_scope(path: str, scopes: list[str]) -> bool:
    normalized = path.rstrip("/")
    return any(normalized == scope.rstrip("/") or normalized.startswith(scope.rstrip("/") + "/") for scope in scopes)


def provider_family(route: dict[str, Any]) -> str:
    value = f"{route['harness']}/{route['model']}".lower()
    if "claude" in value or "anthropic" in value:
        return "anthropic"
    if "codex" in value or "openai" in value or "gpt" in value:
        return "openai"
    return route["harness"]


def derive_review(implementation: dict[str, Any], closeout: dict[str, Any], base: str, head: str) -> dict[str, Any]:
    validate_artifact("implementation", implementation)
    validate_artifact("closeout", closeout)
    expected = {"artifact_id": implementation["artifact_id"], "digest": digest(implementation)}
    if closeout.get("source_implementation") != expected:
        raise HandoffError("HANDOFF_LINEAGE_MISMATCH", "closeout does not cite the implementation handoff")
    changed = closeout.get("changed_paths")
    if not isinstance(changed, list) or not changed or any(not in_scope(path, implementation["allowed_write_paths"]) for path in changed):
        raise HandoffError("HANDOFF_SCOPE_INVALID", "changed paths exceed implementation scope")
    result = {
        "version": 1,
        "artifact_id": "sha256:" + "0" * 64,
        "created_at": timestamp(),
        "repository": implementation["repository"],
        "source_implementation": expected,
        "producer": producer(),
        "base_commit": base,
        "head_commit": head,
        "changed_paths": changed,
        "closeout": {"path": implementation["closeout"]["path"], "digest": digest(closeout), "claims": closeout["claims"]},
        "acceptance_criteria": implementation["acceptance_criteria"],
        "rerun_commands": [item["verification"] for item in implementation["acceptance_criteria"]],
        "required_independence": {"provider_family": provider_family(implementation["route"]), "session": "isolated"},
        "checks": closeout["checks"],
        "verdict": {"path": f".autospec/handoffs/{implementation['issue']['number']}/verdict.json", "schema": "autospec-agent-handoff-result-v1.schema.json"},
    }
    result["artifact_id"] = artifact_id(result)
    validate_artifact("review", result)
    return result


def accept_result(handoff: dict[str, Any], result: dict[str, Any]) -> None:
    validate_artifact("result", result)
    if "required_independence" not in handoff:
        raise HandoffError("HANDOFF_SCHEMA_INVALID", "accept-result requires a review handoff")
    validate_artifact("review", handoff)
    expected = [{"artifact_id": handoff["artifact_id"], "digest": digest(handoff)}]
    if result["role"] != "reviewer" or result["inputs"] != expected:
        raise HandoffError("HANDOFF_LINEAGE_MISMATCH", "review result does not cite the exact review handoff")
    if result["producer"]["session_isolation"] != handoff["required_independence"]["session"]:
        raise HandoffError("HANDOFF_INDEPENDENCE_UNSATISFIED", "review session is not isolated")
    if result["producer"]["provider_family"] == handoff["required_independence"]["provider_family"]:
        raise HandoffError("HANDOFF_INDEPENDENCE_UNSATISFIED", "reviewer provider matches the implementation provider")
    if result["status"] != "pass":
        raise HandoffError("HANDOFF_EVIDENCE_INSUFFICIENT", f"review result status is {result['status']}")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    validate = commands.add_parser("validate")
    validate.add_argument("--kind", choices=sorted(SCHEMA_NAMES), required=True)
    validate.add_argument("--input", type=Path, required=True)
    reconcile = commands.add_parser("reconcile-spec")
    reconcile.add_argument("--proposal", type=Path, required=True)
    reconcile.add_argument("--critique", type=Path, required=True)
    reconcile.add_argument("--repo", type=Path, required=True)
    reconcile.add_argument("--output", type=Path, required=True)
    implementation = commands.add_parser("implementation")
    implementation.add_argument("--spec", type=Path, required=True)
    implementation.add_argument("--issue", type=Path, required=True)
    implementation.add_argument("--output", type=Path, required=True)
    review = commands.add_parser("review")
    review.add_argument("--implementation", type=Path, required=True)
    review.add_argument("--closeout", type=Path, required=True)
    review.add_argument("--base", required=True)
    review.add_argument("--head", required=True)
    review.add_argument("--output", type=Path, required=True)
    accept = commands.add_parser("accept-result")
    accept.add_argument("--handoff", type=Path, required=True)
    accept.add_argument("--result", type=Path, required=True)
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "validate":
            validate_artifact(args.kind, load_json(args.input))
            return 0
        if args.command == "accept-result":
            accept_result(load_json(args.handoff), load_json(args.result))
            return 0
        if args.command == "reconcile-spec":
            result = reconcile_spec(load_json(args.proposal), load_json(args.critique), args.repo)
        elif args.command == "implementation":
            result = derive_implementation(load_json(args.spec), load_json(args.issue))
        else:
            result = derive_review(load_json(args.implementation), load_json(args.closeout), args.base, args.head)
        output_json(args.output, result)
        print(json.dumps({"artifact_id": result["artifact_id"], "output": str(args.output)}, sort_keys=True))
        return 0
    except HandoffError as exc:
        print(f"{exc.category}: {exc}", file=sys.stderr)
        return 3
    except (KeyError, TypeError, ValueError) as exc:
        print(f"HANDOFF_SCHEMA_INVALID: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

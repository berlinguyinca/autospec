#!/usr/bin/env bash
# Resolve autospec governance policy and scrub observatory events by privacy tier.
set -euo pipefail

python3 - "$@" <<'PY'
import argparse
import hashlib
import json
import sys
from pathlib import Path

TIER_ORDER = {"metadata-only": 0, "summary": 1, "evidence": 2, "full-debug": 3}
PROJECT_DEFAULTS = {
    "open-source": {"privacy_tier": "summary", "raw_logs_allowed": False, "policy_id": "builtin-open-source-default"},
    "private-personal": {"privacy_tier": "evidence", "raw_logs_allowed": False, "policy_id": "builtin-private-personal-default"},
    "private-company": {"privacy_tier": "summary", "raw_logs_allowed": False, "policy_id": "builtin-private-company-default"},
    "client-project": {"privacy_tier": "metadata-only", "raw_logs_allowed": False, "policy_id": "builtin-client-project-default"},
    "research": {"privacy_tier": "summary", "raw_logs_allowed": False, "policy_id": "builtin-research-default"},
    "sandbox": {"privacy_tier": "evidence", "raw_logs_allowed": True, "policy_id": "builtin-sandbox-default"},
}
GOVERNANCE_FILES = {
    "open-source": "open-source-maintainer-default.yml",
    "private-personal": "private-personal-default.yml",
    "private-company": "private-company-default.yml",
    "client-project": "client-project-default.yml",
    "research": "research-default.yml",
    "sandbox": "sandbox-default.yml",
}


def fail(message: str, code: int = 1) -> None:
    print(message, file=sys.stderr)
    print(message)
    raise SystemExit(code)


def parse_scalar(value: str):
    value = value.strip().strip('"').strip("'")
    if value.lower() == "true":
        return True
    if value.lower() == "false":
        return False
    return value


def read_simple_yaml(path: Path) -> dict:
    data = {}
    if not path.exists():
        return data
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].rstrip()
        if not line.strip() or line.startswith((" ", "\t", "-")):
            continue
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        key = key.strip()
        value = value.strip()
        if value == "":
            continue
        data[key] = parse_scalar(value)
    return data


def normalize_policy(data: dict, source: str, project_class: str | None = None) -> dict:
    cls = str(data.get("project_class") or project_class or "sandbox")
    defaults = PROJECT_DEFAULTS.get(cls, PROJECT_DEFAULTS["sandbox"])
    tier = str(data.get("privacy_tier") or defaults["privacy_tier"])
    if tier not in TIER_ORDER:
        fail(f"unsupported privacy tier: {tier}")
    return {
        "policy_source": source,
        "policy_id": str(data.get("policy_id") or defaults["policy_id"]),
        "policy_version": str(data.get("policy_version") or data.get("version") or "builtin-safe-fallback-v1"),
        "policy_digest": str(data.get("policy_digest") or ""),
        "project_class": cls,
        "privacy_tier": tier,
        "raw_logs_allowed": bool(data.get("raw_logs_allowed", defaults["raw_logs_allowed"])),
    }


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return "sha256:" + h.hexdigest()


def resolve_policy(args):
    repo = Path(args.repo)
    trace: list[str] = []

    local = repo / ".autospec" / "autonomous.yml"
    if local.exists():
        trace.append("repo-local:.autospec/autonomous.yml:hit")
        policy = normalize_policy(read_simple_yaml(local), "repo-local", args.project_class)
        policy["policy_resolution_trace"] = trace
        return policy
    trace.append("repo-local:.autospec/autonomous.yml:miss")

    assignment_path = Path(args.observatory_assignment) if args.observatory_assignment else repo / ".autospec" / "observatory-assignment.yml"
    if assignment_path.exists():
        trace.append(f"observatory-assignment:{assignment_path.name}:hit")
        policy = normalize_policy(read_simple_yaml(assignment_path), "observatory-assignment", args.project_class)
        policy["policy_resolution_trace"] = trace
        return policy
    trace.append("observatory-assignment:miss")

    project_class = args.project_class or "sandbox"
    if args.governance_dir:
        policy_file = Path(args.governance_dir) / "policies" / GOVERNANCE_FILES.get(project_class, "sandbox-default.yml")
        if policy_file.exists():
            trace.append(f"governance-default:{policy_file.name}:hit")
            digest = sha256_file(policy_file)
            if args.expected_policy_digest and args.expected_policy_digest != digest:
                fail(f"policy digest mismatch: expected {args.expected_policy_digest} got {digest}")
            data = read_simple_yaml(policy_file)
            data["policy_digest"] = digest
            policy = normalize_policy(data, "governance-default", project_class)
            policy["policy_resolution_trace"] = trace
            return policy
        trace.append(f"governance-default:{policy_file.name}:miss")
    else:
        trace.append("governance-default:not-configured")

    trace.append("built-in-safe-fallback:hit")
    policy = normalize_policy({}, "built-in-safe-fallback", project_class)
    policy["policy_digest"] = "sha256:builtin-safe-fallback"
    policy["policy_resolution_trace"] = trace
    return policy


def ensure_tier_allowed(policy_tier: str, api_key_tier: str) -> None:
    if TIER_ORDER[policy_tier] > TIER_ORDER[api_key_tier]:
        fail(f"event exceeds api key privacy tier: policy={policy_tier} api_key={api_key_tier}")


def scrub_event(event: dict, policy: dict, allow_full_debug_raw_logs: bool) -> dict:
    tier = policy["privacy_tier"]
    raw_present = "raw_logs" in event and event.get("raw_logs") not in (None, "")
    if tier == "full-debug" and raw_present:
        if not policy.get("raw_logs_allowed") or not allow_full_debug_raw_logs:
            fail("full-debug raw logs rejected: fixture key must allow raw log upload")
    base = {
        "event_type": event.get("event_type"),
        "repo": event.get("repo"),
        "policy_id": policy["policy_id"],
        "policy_version": policy["policy_version"],
        "policy_digest": policy["policy_digest"],
        "policy_source": policy["policy_source"],
        "policy_resolution_trace": policy["policy_resolution_trace"],
        "project_class": policy["project_class"],
        "privacy_tier": tier,
    }
    if tier == "metadata-only":
        return {k: v for k, v in base.items() if v not in (None, "")}

    if "summary" in event:
        base["summary"] = event["summary"]
    if tier in ("evidence", "full-debug"):
        for key in ("evidence", "artifact_summary"):
            if key in event:
                base[key] = event[key]
    if tier == "full-debug":
        for key in ("artifact_details", "raw_logs", "debug"):
            if key in event:
                base[key] = event[key]
    return {k: v for k, v in base.items() if v not in (None, "")}


def main() -> None:
    parser = argparse.ArgumentParser(description="Resolve autospec policy and scrub observatory events")
    parser.add_argument("--repo", default=".")
    parser.add_argument("--project-class", choices=sorted(PROJECT_DEFAULTS), default=None)
    parser.add_argument("--observatory-assignment", default=None)
    parser.add_argument("--governance-dir", default=None)
    parser.add_argument("--expected-policy-digest", default=None)
    parser.add_argument("--event-file", default=None)
    parser.add_argument("--api-key-privacy-tier", choices=sorted(TIER_ORDER, key=TIER_ORDER.get), default="full-debug")
    parser.add_argument("--allow-full-debug-raw-logs", action="store_true")
    args = parser.parse_args()

    policy = resolve_policy(args)
    ensure_tier_allowed(policy["privacy_tier"], args.api_key_privacy_tier)

    result = dict(policy)
    if args.event_file:
        event_path = Path(args.event_file)
        try:
            event = json.loads(event_path.read_text(encoding="utf-8"))
        except Exception as exc:
            fail(f"invalid event json: {event_path}: {exc}")
        result["scrubbed_event"] = scrub_event(event, policy, args.allow_full_debug_raw_logs)

    print(json.dumps(result, sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    main()
PY

#!/usr/bin/env bash
# scripts/lint-issue-safety.sh - issue intent safety gate.

set -eu

JSON_MODE=0
ACTOR=""
TITLE=""
CONFIG_PATH=".autospec/autospec.yml"
BODY_FILE=""

usage() {
    printf 'Usage: scripts/lint-issue-safety.sh [--json] [--actor LOGIN] [--title TITLE] [--config PATH] <body-file>\n'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --json)
            JSON_MODE=1
            shift
            ;;
        --actor)
            if [ $# -lt 2 ]; then
                usage >&2
                exit 64
            fi
            ACTOR="${2:-}"
            shift 2
            ;;
        --title)
            if [ $# -lt 2 ]; then
                usage >&2
                exit 64
            fi
            TITLE="${2:-}"
            shift 2
            ;;
        --config)
            if [ $# -lt 2 ]; then
                usage >&2
                exit 64
            fi
            CONFIG_PATH="${2:-}"
            shift 2
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        -*)
            printf 'lint-issue-safety.sh: unknown option: %s\n' "$1" >&2
            exit 64
            ;;
        *)
            if [ -n "$BODY_FILE" ]; then
                printf 'lint-issue-safety.sh: multiple body files are not supported\n' >&2
                exit 64
            fi
            BODY_FILE="$1"
            shift
            ;;
    esac
done

if [ -z "$BODY_FILE" ] || [ ! -f "$BODY_FILE" ]; then
    printf 'lint-issue-safety.sh: body file not found: %s\n' "$BODY_FILE" >&2
    exit 64
fi

python3 - "$JSON_MODE" "$ACTOR" "$TITLE" "$CONFIG_PATH" "$BODY_FILE" <<'PY'
import json
import re
import sys
from pathlib import Path

json_mode = sys.argv[1] == "1"
actor = sys.argv[2]
title = sys.argv[3]
config_path = Path(sys.argv[4])
body_path = Path(sys.argv[5])
text = title + "\n" + body_path.read_text(encoding="utf-8", errors="replace")

DEFAULT_POLICY = {
    "block_patterns": [
        {"id": "production-data-destruction", "patterns": [r"(?i)delete .*production", r"(?i)drop .*prod(uction)? .*database"]},
        {"id": "secret-exfiltration", "patterns": [r"(?i)(dump|print|exfiltrate|send).*secret", r"(?i)(aws|github|stripe).*token"]},
        {"id": "instruction-bypass", "patterns": [r"(?i)ignore (all )?(previous|system|developer|agent) instructions", r"(?i)bypass (ci|tests|hooks|review|guardian)"]},
        {"id": "destructive-shell", "patterns": [r"rm -rf /", r"(?i)curl .*\| *(sh|bash)"]},
    ],
    "ambiguous_patterns": [
        {"id": "vague-data-cleanup", "patterns": [r"(?i)clean (old|bad|stale)? ?data"]},
        {"id": "weaken-security-control", "patterns": [r"(?i)(relax|disable|remove).*security", r"(?i)(relax|disable|remove).*(auth|audit|logging)"]},
        {"id": "production-or-infra-touch", "patterns": [r"(?i)(production|prod|billing|payments|migration|terraform|iam|kms)"]},
    ],
    "trusted_actors": [{"login": "berlinguyinca", "allowed_risk": ["test_data_reset", "fixture_regeneration", "local_dev_cleanup", "documented_migration_replay"]}],
    "never_bypass": ["secret-exfiltration", "instruction-bypass", "production-data-destruction"],
}


def load_policy():
    policy = {
        "block_patterns": list(DEFAULT_POLICY["block_patterns"]),
        "ambiguous_patterns": list(DEFAULT_POLICY["ambiguous_patterns"]),
        "trusted_actors": list(DEFAULT_POLICY["trusted_actors"]),
        "never_bypass": list(DEFAULT_POLICY["never_bypass"]),
    }
    if not config_path.exists():
        return policy
    try:
        import yaml

        data = yaml.safe_load(config_path.read_text(encoding="utf-8")) or {}
        gate = (data.get("safety") or {}).get("issue_intent_gate") or {}
        for key in ("block_patterns", "ambiguous_patterns", "trusted_actors"):
            if isinstance(gate.get(key), list):
                policy[key].extend(gate[key])
        rules = gate.get("trusted_actor_rules") or {}
        if isinstance(rules.get("never_bypass"), list):
            policy["never_bypass"].extend(rules["never_bypass"])
    except Exception:
        pass
    return policy


def add_matches(findings, severity, rows):
    for row in rows:
        rid = str(row.get("id", "unnamed-rule"))
        for pattern in row.get("patterns", []):
            try:
                if re.search(pattern, text):
                    findings.append({"severity": severity, "rule_id": rid, "pattern": pattern})
                    break
            except re.error:
                findings.append({"severity": "block", "rule_id": "invalid-policy-regex", "pattern": str(pattern)})


def is_trusted_test_reset(policy):
    if not actor:
        return False
    trusted = any(row.get("login") == actor for row in policy.get("trusted_actors", []))
    if not trusted:
        return False
    has_reset = re.search(r"(?i)(delete|reset|repopulate).*test .*database|test database.*(delete|reset|repopulate)", text)
    scoped = re.search(r"(?i)\b(test|local|fixture|dev)\b", text) and re.search(r"(?i)production.*out of scope|production, staging", text)
    return bool(has_reset and scoped)


policy = load_policy()
findings = []
add_matches(findings, "block", policy["block_patterns"])
add_matches(findings, "ambiguous", policy["ambiguous_patterns"])

if is_trusted_test_reset(policy):
    never = set(policy.get("never_bypass", []))
    blocking = [f for f in findings if f["severity"] == "block" and f["rule_id"] in never]
    if not blocking:
        findings = [{"severity": "info", "rule_id": "trusted:test_data_reset", "pattern": actor}]

blocking = [f for f in findings if f["severity"] == "block"]
ambiguous = [f for f in findings if f["severity"] == "ambiguous"]
if blocking:
    decision = "SAFETY_BLOCK"
    exit_code = 2
elif ambiguous:
    decision = "SAFETY_AMBIGUOUS"
    exit_code = 1
else:
    decision = "SAFETY_PASS"
    exit_code = 0

payload = {
    "decision": decision,
    "findings": findings,
    "actor": actor or None,
    "trusted": any(f["rule_id"].startswith("trusted:") for f in findings),
}
if json_mode:
    print(json.dumps(payload, separators=(",", ":")))
else:
    print(decision)
    for f in findings:
        print(f"RULE_ID: {f['severity']}: {f['rule_id']}: matched {f['pattern']}")
sys.exit(exit_code)
PY

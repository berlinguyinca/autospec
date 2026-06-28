#!/usr/bin/env bash
# scripts/autospec-bot-state-init.sh — write inert local Autospec bot state.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-bot-state-init.sh [--repo-root <dir>] [--force]

Inputs:
  .autospec/reports/issue-plan.json

Writes:
  .autospec/state/bot-control-plane.json
  .autospec/state/autonomous-backlog.json
  .autospec/templates/autonomous-issue.md
EOF
}

die() {
    printf 'autospec-bot-state-init: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
FORCE=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --force) FORCE=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$FORCE" <<'PY'
import json
import os
import sys

repo_root = os.path.realpath(sys.argv[1])
force = sys.argv[2] == "1"
state_dir = os.path.join(repo_root, ".autospec", "state")
reports_dir = os.path.join(repo_root, ".autospec", "reports")
templates_dir = os.path.join(repo_root, ".autospec", "templates")
control_path = os.path.join(state_dir, "bot-control-plane.json")
backlog_path = os.path.join(state_dir, "autonomous-backlog.json")
template_path = os.path.join(templates_dir, "autonomous-issue.md")
issue_plan_path = os.path.join(reports_dir, "issue-plan.json")


def load_json(path, default):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return json.load(fh)
    except Exception:
        return default


def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
        fh.write("\n")


if os.path.exists(control_path) and not force:
    print("bot state init: FAIL")
    print("bot-control-plane.json already exists; rerun with --force to regenerate local dry-run state.")
    sys.exit(1)

issue_plan = load_json(issue_plan_path, {"issues": []})
issues = issue_plan.get("issues", []) if isinstance(issue_plan.get("issues"), list) else []
os.makedirs(state_dir, exist_ok=True)
os.makedirs(templates_dir, exist_ok=True)

control = {
    "version": 1,
    "mode": "dry_run",
    "engine": "autospec",
    "law": "autospec-constitution",
    "playbooks": "autospec-baselines",
    "write_permissions": {
        "github": False,
        "branches": False,
        "pull_requests": False,
        "implementation": False,
    },
    "allowed_artifact_roots": [
        ".autospec/state",
        ".autospec/reports",
        ".autospec/backlog",
        ".autospec/templates",
    ],
    "queue_states": ["ready", "blocked", "planned", "in_progress", "done", "skipped"],
    "transition_rules": [
        {"from": "ready", "to": "planned", "allowed_in_dry_run": True},
        {"from": "planned", "to": "in_progress", "allowed_in_dry_run": False},
        {"from": "in_progress", "to": "done", "allowed_in_dry_run": False},
        {"from": "ready", "to": "blocked", "allowed_in_dry_run": True},
        {"from": "blocked", "to": "ready", "allowed_in_dry_run": True},
        {"from": "ready", "to": "skipped", "allowed_in_dry_run": True},
    ],
    "stop_conditions": [
        "missing required local report input",
        "manual state exists without --force",
        "requested action would write to GitHub",
        "requested action would create branch, PR, or implementation changes",
    ],
}
ready = [
    {
        "number": issue.get("number"),
        "title": issue.get("title"),
        "priority": issue.get("priority"),
        "risk": issue.get("risk"),
        "draft_path": issue.get("draft_path"),
        "state": "ready",
        "dependencies": issue.get("dependencies", []),
    }
    for issue in issues
]
backlog = {
    "version": 1,
    "mode": "dry_run",
    "source": ".autospec/reports/issue-plan.json",
    "queue": {
        "ready": ready,
        "blocked": [],
        "planned": [],
        "in_progress": [],
        "done": [],
        "skipped": [],
    },
}
write_json(control_path, control)
write_json(backlog_path, backlog)

template = """# {{title}}

## Control Plane

- Mode: dry_run
- GitHub writes: false
- Branch creation: false
- PR creation: false
- Autonomous implementation: false

## Required Fields

- Source gap
- Evidence
- Confidence
- Priority
- Risk
- Dependencies
- Acceptance criteria
- Suggested validation command

## State Transitions

`ready` -> `planned` is allowed in dry-run mode.
`planned` -> `in_progress` requires a future implementation-mode gate.
"""
with open(template_path, "w", encoding="utf-8") as fh:
    fh.write(template)

print("bot state init: PASS")
print("state: .autospec/state/bot-control-plane.json, .autospec/state/autonomous-backlog.json")
print("template: .autospec/templates/autonomous-issue.md")
PY

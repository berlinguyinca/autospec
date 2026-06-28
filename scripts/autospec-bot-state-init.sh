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
  .autospec/state/control-labels.yml
  .autospec/reports/control-labels.md
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
control_labels_path = os.path.join(state_dir, "control-labels.yml")
control_labels_report_path = os.path.join(reports_dir, "control-labels.md")
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


def yaml_scalar(value):
    text = str(value).replace("'", "''")
    return f"'{text}'"


def yaml_list(values, indent):
    if not values:
        return [" " * indent + "[]"]
    return [" " * indent + f"- {yaml_scalar(value)}" for value in values]


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
control_labels = [
    {
        "name": "autospec:managed",
        "purpose": "Marks backlog items governed by Autospec dry-run/autonomous control rules.",
        "may_apply": ["autospec planner", "operator"],
        "may_remove": ["operator", "autospec cleanup after item leaves Autospec control"],
        "compatible_labels": ["autospec:discovered", "autospec:active", "autospec:paused", "autospec:blocked", "autospec:needs-review", "autospec:follow-up"],
        "incompatible_labels": [],
        "state_machine_effect": "Places the item under Autospec control-plane accounting.",
    },
    {
        "name": "autospec:discovered",
        "purpose": "Identifies issues drafted from local metadata, baseline, or constitution gap reports.",
        "may_apply": ["autospec planner"],
        "may_remove": ["operator"],
        "compatible_labels": ["autospec:managed", "autospec:follow-up", "autospec:self-improvement"],
        "incompatible_labels": [],
        "state_machine_effect": "Keeps provenance as discovered backlog rather than operator-authored work.",
    },
    {
        "name": "autospec:active",
        "purpose": "Marks the item currently selected for an execution lane.",
        "may_apply": ["future autospec runner", "operator"],
        "may_remove": ["future autospec runner", "operator"],
        "compatible_labels": ["autospec:managed", "autospec:needs-review", "autospec:risk-high"],
        "incompatible_labels": ["autospec:paused", "autospec:blocked", "autospec:stuck"],
        "state_machine_effect": "Moves queue state to in_progress when implementation mode exists.",
    },
    {
        "name": "autospec:paused",
        "purpose": "Stops automatic progress on an otherwise managed item.",
        "may_apply": ["operator", "future autospec stop command"],
        "may_remove": ["operator", "future autospec resume command"],
        "compatible_labels": ["autospec:managed", "autospec:resume", "autospec:needs-guidance"],
        "incompatible_labels": ["autospec:active"],
        "state_machine_effect": "Moves queue state to blocked and prevents automatic selection.",
    },
    {
        "name": "autospec:blocked",
        "purpose": "Records a concrete blocker that prevents safe progress.",
        "may_apply": ["autospec planner", "future autospec runner", "operator"],
        "may_remove": ["operator", "future autospec runner after blocker clears"],
        "compatible_labels": ["autospec:managed", "autospec:needs-guidance", "autospec:risk-high"],
        "incompatible_labels": ["autospec:active", "autospec:resume"],
        "state_machine_effect": "Moves queue state to blocked until the blocker is resolved.",
    },
    {
        "name": "autospec:stuck",
        "purpose": "Escalates repeated failure or no-progress loops.",
        "may_apply": ["future autospec runner", "operator"],
        "may_remove": ["operator"],
        "compatible_labels": ["autospec:managed", "autospec:blocked", "autospec:needs-guidance"],
        "incompatible_labels": ["autospec:active", "autospec:resume"],
        "state_machine_effect": "Freezes automatic retries and requires operator guidance.",
    },
    {
        "name": "autospec:needs-guidance",
        "purpose": "Signals that human direction is required before proceeding.",
        "may_apply": ["autospec planner", "future autospec runner", "operator"],
        "may_remove": ["operator", "future autospec runner after guidance is recorded"],
        "compatible_labels": ["autospec:managed", "autospec:blocked", "autospec:paused"],
        "incompatible_labels": ["autospec:guidance-provided", "autospec:active"],
        "state_machine_effect": "Blocks selection until guidance is provided.",
    },
    {
        "name": "autospec:guidance-provided",
        "purpose": "Records that requested guidance has been supplied.",
        "may_apply": ["operator", "future autospec listener"],
        "may_remove": ["future autospec runner after consuming guidance", "operator"],
        "compatible_labels": ["autospec:managed", "autospec:resume"],
        "incompatible_labels": ["autospec:needs-guidance", "autospec:stuck"],
        "state_machine_effect": "Allows a blocked guidance item to return to ready/resume.",
    },
    {
        "name": "autospec:resume",
        "purpose": "Requests that paused or guidance-complete work re-enter the ready queue.",
        "may_apply": ["operator", "future autospec resume command"],
        "may_remove": ["future autospec runner after queue transition", "operator"],
        "compatible_labels": ["autospec:managed", "autospec:paused", "autospec:guidance-provided"],
        "incompatible_labels": ["autospec:blocked", "autospec:stuck", "autospec:active"],
        "state_machine_effect": "Moves eligible blocked/paused items back to ready.",
    },
    {
        "name": "autospec:needs-review",
        "purpose": "Marks work that requires reviewer attention before completion.",
        "may_apply": ["autospec planner", "future autospec runner", "operator"],
        "may_remove": ["operator", "future reviewer gate"],
        "compatible_labels": ["autospec:managed", "autospec:active", "autospec:risk-high"],
        "incompatible_labels": [],
        "state_machine_effect": "Prevents done transition until review evidence exists.",
    },
    {
        "name": "autospec:architecture",
        "purpose": "Identifies architecture-sensitive backlog items.",
        "may_apply": ["autospec planner", "architect reviewer", "operator"],
        "may_remove": ["operator"],
        "compatible_labels": ["autospec:managed", "autospec:risk-high", "autospec:needs-review"],
        "incompatible_labels": [],
        "state_machine_effect": "Raises review requirements and may place item later in priority order.",
    },
    {
        "name": "autospec:risk-high",
        "purpose": "Marks high-risk work requiring stricter gates.",
        "may_apply": ["autospec planner", "future risk classifier", "operator"],
        "may_remove": ["operator", "future risk classifier after reclassification"],
        "compatible_labels": ["autospec:managed", "autospec:architecture", "autospec:needs-review"],
        "incompatible_labels": [],
        "state_machine_effect": "Requires review before execution and completion transitions.",
    },
    {
        "name": "autospec:self-improvement",
        "purpose": "Identifies work that improves Autospec itself.",
        "may_apply": ["autospec planner", "operator"],
        "may_remove": ["operator"],
        "compatible_labels": ["autospec:managed", "autospec:discovered", "autospec:follow-up"],
        "incompatible_labels": [],
        "state_machine_effect": "Routes work through self-improvement accounting and memory capture.",
    },
    {
        "name": "autospec:follow-up",
        "purpose": "Marks deferred work discovered while handling another item.",
        "may_apply": ["autospec planner", "future autospec runner", "operator"],
        "may_remove": ["operator", "future backlog grooming"],
        "compatible_labels": ["autospec:managed", "autospec:discovered", "autospec:self-improvement"],
        "incompatible_labels": ["autospec:active"],
        "state_machine_effect": "Keeps item in backlog but deprioritizes until dependencies and primary work settle.",
    },
]
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

with open(control_labels_path, "w", encoding="utf-8") as fh:
    fh.write("version: 1\n")
    fh.write("labels:\n")
    for label in control_labels:
        fh.write(f"  {label['name']}:\n")
        fh.write(f"    purpose: {yaml_scalar(label['purpose'])}\n")
        fh.write("    may_apply:\n")
        fh.write("\n".join(yaml_list(label["may_apply"], 6)) + "\n")
        fh.write("    may_remove:\n")
        fh.write("\n".join(yaml_list(label["may_remove"], 6)) + "\n")
        fh.write("    compatible_labels:\n")
        fh.write("\n".join(yaml_list(label["compatible_labels"], 6)) + "\n")
        fh.write("    incompatible_labels:\n")
        fh.write("\n".join(yaml_list(label["incompatible_labels"], 6)) + "\n")
        fh.write(f"    state_machine_effect: {yaml_scalar(label['state_machine_effect'])}\n")

label_lines = [
    "# Control Label Taxonomy",
    "",
    "Local v0 taxonomy for Autospec-managed backlog state. This report is documentation only; it does not create or mutate GitHub labels.",
    "",
    "| Label | Purpose | May apply | May remove | Compatible | Incompatible | State-machine effect |",
    "| --- | --- | --- | --- | --- | --- | --- |",
]
for label in control_labels:
    label_lines.append(
        f"| `{label['name']}` | {label['purpose']} | {', '.join(label['may_apply'])} | "
        f"{', '.join(label['may_remove'])} | {', '.join(label['compatible_labels']) or 'none'} | "
        f"{', '.join(label['incompatible_labels']) or 'none'} | {label['state_machine_effect']} |"
    )
with open(control_labels_report_path, "w", encoding="utf-8") as fh:
    fh.write("\n".join(label_lines))
    fh.write("\n")

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
print("labels: .autospec/state/control-labels.yml, .autospec/reports/control-labels.md")
print("template: .autospec/templates/autonomous-issue.md")
PY

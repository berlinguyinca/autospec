#!/usr/bin/env bash
# scripts/autospec-mvp-status.sh — summarize Autospec Constitution MVP readiness.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-mvp-status.sh [--repo-root DIR]

Writes .autospec/reports/mvp-status.json and .autospec/reports/mvp-status.md.
EOF
}

die() {
    printf 'autospec-mvp-status: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" <<'PY'
import json
import os
import sys

root = os.path.realpath(sys.argv[1])
reports = os.path.join(root, ".autospec", "reports")
state = os.path.join(root, ".autospec", "state")
script_root = os.path.dirname(os.path.realpath(__file__)) if "__file__" in globals() else os.path.join(root, "scripts")


def exists(path):
    return os.path.exists(os.path.join(root, path))


def load(path, default):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            data = json.load(fh)
        return data if isinstance(data, dict) else default
    except Exception:
        return default


def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
        fh.write("\n")


def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text.rstrip() + "\n")


rules = load(os.path.join(state, "rule-check-results.json"), load(os.path.join(reports, "rule-check-results.json"), {"results": []}))
published = load(os.path.join(state, "published-issues.json"), {"issues": []})
status = {
    "schema": 1,
    "engine_capabilities": {
        "policy_sources": exists(".autospec/policy-sources.lock.json") or exists(".autospec/state/policy-sources.json"),
        "digital_twin": exists(".autospec/state/digital-twin.json"),
        "issue_plan_v3": exists(".autospec/reports/issue-plan-v3.json"),
        "worker": exists("scripts/autospec-worker-v1.sh"),
        "verifier": exists("scripts/autospec-verify-worker-pr.sh"),
        "supervisor": exists("scripts/autospec-supervisor-cycle.sh"),
        "onboarding": exists("scripts/autospec-onboard-existing-repo.sh"),
        "bootstrap": exists("scripts/autospec-bootstrap-new-project.sh"),
        "ai_nlai_scaffolds": exists("scripts/autospec-generate-ai-nlai-scaffold.sh"),
    },
    "policy_source_status": "present" if exists(".autospec/state/policy-sources.json") else "unknown",
    "baseline_source_status": "present" if exists(".autospec/reports/baseline-composition.json") else "unknown",
    "digital_twin_status": "present" if exists(".autospec/state/digital-twin.json") else "missing",
    "structured_rule_audit_status": "present" if rules.get("results") else "missing",
    "backlog_publishing_status": f"{sum(1 for item in published.get('issues', []) if item.get('plan_version') == 'v3')} v3 issues published",
    "known_limitations": "docs/KNOWN_LIMITATIONS.md",
    "recommended_next_milestones": [
        "Dogfood existing-repo onboarding on a real target repository.",
        "Run one dry-run structured-rule supervisor cycle.",
        "Expand structured Constitution coverage as policy repos mature.",
    ],
}
write_json(os.path.join(reports, "mvp-status.json"), status)
md = "\n".join([
    "# Autospec Constitution MVP Status",
    "",
    "## Engine capabilities",
    "",
    "\n".join(f"- {key}: `{str(value).lower()}`" for key, value in status["engine_capabilities"].items()),
    "",
    "## Policy source status",
    "",
    status["policy_source_status"],
    "",
    "## Baseline source status",
    "",
    status["baseline_source_status"],
    "",
    "## Digital Twin status",
    "",
    status["digital_twin_status"],
    "",
    "## Structured rule audit status",
    "",
    status["structured_rule_audit_status"],
    "",
    "## Backlog publishing status",
    "",
    status["backlog_publishing_status"],
    "",
    "## Worker/verifier/supervisor status",
    "",
    "- Worker, verifier, promotion, supervisor, loop, status, and guidance commands are local/operator invoked.",
    "",
    "## Onboarding support",
    "",
    "- Existing repository onboarding command is available.",
    "",
    "## Bootstrap support",
    "",
    "- New project bootstrap command is available.",
    "",
    "## AI/NLAI scaffold support",
    "",
    "- AI/NLAI scaffold generator is available.",
    "",
    "## Safety guarantees",
    "",
    "- No GitHub Actions or schedulers.",
    "- Dry-run first.",
    "- Confirm required for GitHub writes.",
    "- No auto-merge or self-approval.",
    "",
    "## Known limitations",
    "",
    "- See `docs/KNOWN_LIMITATIONS.md`.",
    "",
    "## Recommended next milestones",
    "",
    "\n".join(f"- {item}" for item in status["recommended_next_milestones"]),
])
write_text(os.path.join(reports, "mvp-status.md"), md)
print("mvp status: wrote reports")
PY

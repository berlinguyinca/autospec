#!/usr/bin/env bash
# scripts/autospec-start.sh — local operator entrypoint for Autospec MVP flows.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-start.sh [--repo-root DIR] [--dry-run|--confirm] [--mode existing|new] [--name NAME] [--profiles a,b]

This command never writes GitHub data. Confirm is accepted for consistency but
the command only writes local start-plan reports.
EOF
}

die() {
    printf 'autospec-start: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
CONFIRM=0
MODE="auto"
NAME=""
PROFILES=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        --mode) [ "$#" -ge 2 ] || die "--mode requires a value"; MODE="$2"; shift 2 ;;
        --name) [ "$#" -ge 2 ] || die "--name requires a value"; NAME="$2"; shift 2 ;;
        --profiles) [ "$#" -ge 2 ] || die "--profiles requires a value"; PROFILES="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$CONFIRM" "$MODE" "$NAME" "$PROFILES" <<'PY'
import json
import os
import sys

root = os.path.realpath(sys.argv[1])
confirm = sys.argv[2] == "1"
mode, name, profiles = sys.argv[3:6]
reports = os.path.join(root, ".autospec", "reports")
state = os.path.join(root, ".autospec", "state")


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


def has_existing_source():
    for base, dirs, files in os.walk(root):
        rel = os.path.relpath(base, root)
        if any(part in {".git", ".autospec", "node_modules"} for part in rel.split(os.sep)):
            dirs[:] = []
            continue
        if files:
            if any(f.endswith((".py", ".js", ".ts", ".tsx", ".go", ".rs", ".java", ".md")) for f in files):
                return True
    return False


onboarding = load(os.path.join(state, "onboarding.json"), {})
published = load(os.path.join(state, "published-issues.json"), {"issues": []})
handover = load(os.path.join(state, "stuck-handovers.json"), {"handovers": []})
promotions = os.path.isdir(os.path.join(state, "promotions")) and os.listdir(os.path.join(state, "promotions")) or []
issue_plan_v3 = load(os.path.join(reports, "issue-plan-v3.json"), {"issues": []})
recommendations = []
reason = ""
if mode == "new" or (mode == "auto" and not has_existing_source()):
    reason = "repository appears empty/new"
    cmd = "bash scripts/autospec-bootstrap-new-project.sh --dry-run"
    if name:
        cmd += f" --name {name}"
    if profiles:
        cmd += f" --profiles {profiles}"
    recommendations.append(cmd)
elif mode == "existing" or not onboarding:
    reason = "repository has existing source/docs/tests and no completed onboarding state"
    recommendations.append("bash scripts/autospec-onboard-existing-repo.sh --dry-run" + (f" --profiles {profiles}" if profiles else ""))
else:
    reason = "onboarding exists"
    recommendations.append("bash scripts/autospec-constitution-audit.sh")
if issue_plan_v3.get("issues"):
    recommendations.append("bash scripts/autospec-audit-to-backlog.sh --dry-run")
if any(item.get("plan_version") == "v3" for item in published.get("issues", [])):
    recommendations.append("bash scripts/autospec-supervisor-cycle.sh --dry-run --next")
if any(item.get("state") in {"stuck", "needs-guidance"} for item in handover.get("handovers", [])):
    recommendations.append("bash scripts/autospec-sync-guidance.sh --dry-run")
    recommendations.append("autospec-guide: What is stuck?")
if promotions:
    recommendations.append("bash scripts/autospec-verify-worker-pr.sh --dry-run --pr <number>")
    recommendations.append("bash scripts/autospec-promote-pr.sh --dry-run --pr <number>")
if not recommendations:
    recommendations.append("bash scripts/autospec-autonomy-status.sh")

plan = {
    "schema": 1,
    "mode": "confirm" if confirm else "dry_run",
    "selected_mode": mode,
    "reason": reason,
    "github_writes": False,
    "recommendations": recommendations,
}
write_json(os.path.join(reports, "start-plan.json"), plan)
write_text(os.path.join(reports, "start-plan.md"), "\n".join([
    "# Autospec Start Plan",
    "",
    f"Mode: `{'confirm' if confirm else 'dry_run'}`",
    "",
    "## Recommendation reason",
    "",
    reason,
    "",
    "## Next recommended commands",
    "",
    "\n".join(f"- `{cmd}`" for cmd in recommendations),
    "",
    "## Safety",
    "",
    "- No GitHub Actions, scheduler, merge, approval, or GitHub write is performed.",
]))
print("start: wrote start plan")
PY

#!/usr/bin/env bash
# scripts/autospec-autonomy-budget.sh — local autonomy budget evaluator.

set -eu

REPO_ROOT="$(pwd)"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) REPO_ROOT="$2"; shift 2 ;;
        -h|--help) echo "Usage: autospec-autonomy-budget.sh [--repo-root DIR]"; exit 0 ;;
        *) printf 'autospec-autonomy-budget: unknown arg: %s\n' "$1" >&2; exit 2 ;;
    esac
done

[ -d "$REPO_ROOT" ] || { printf 'autospec-autonomy-budget: repo root missing: %s\n' "$REPO_ROOT" >&2; exit 2; }
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" <<'PY'
import glob, json, os, re, sys

root = os.path.realpath(sys.argv[1])
reports = os.path.join(root, ".autospec", "reports")
state = os.path.join(root, ".autospec", "state")
config = os.path.join(root, ".autospec", "autospec.yml")

DEFAULTS = {
    "max_cycles_per_loop": 3,
    "max_worker_prs_per_loop": 2,
    "max_stuck_items_per_loop": 2,
    "max_verifier_failures_per_loop": 2,
    "max_remediation_attempts_per_issue": 1,
    "max_changed_files_per_issue": 8,
    "max_changed_lines_per_issue": 300,
    "max_open_autospec_prs": 5,
    "max_active_issues": 3,
}

def load(path, default):
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

def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text.rstrip() + "\n")

def config_limits():
    limits = dict(DEFAULTS)
    if os.path.exists(config):
        text = open(config, encoding="utf-8").read()
        for key in DEFAULTS:
            match = re.search(r"^\s*" + re.escape(key) + r":\s*([0-9]+)\s*$", text, re.M)
            if match:
                limits[key] = int(match.group(1))
    return limits

def status(used, limit):
    if limit is None:
        return "unknown"
    if used >= limit:
        return "exhausted"
    if limit > 0 and used / limit >= 0.8:
        return "near_limit"
    return "ok"

limits = config_limits()
published = load(os.path.join(state, "published-issues.json"), {"issues": []})
handover = load(os.path.join(state, "stuck-handovers.json"), {"handovers": []})
supervisor = load(os.path.join(state, "supervisor-runs.json"), {"runs": []})
loops = load(os.path.join(state, "supervisor-loop-runs.json"), {"runs": []})
verifications = [load(path, {}) for path in sorted(glob.glob(os.path.join(state, "verifications", "*.json")))]

active_issues = sum(1 for item in published.get("issues", []) if item.get("state", "open") == "open")
open_prs = len(verifications)
stuck_items = sum(1 for item in handover.get("handovers", []) if item.get("state") in {"needs-guidance", "stuck"})
verifier_failures = sum(1 for item in verifications if item.get("verdict") in {"needs_changes", "blocked", "needs_guidance"})

rows = [
    ("max_cycles_per_loop", len(loops.get("runs", [])), limits["max_cycles_per_loop"], ["state/supervisor-loop-runs.json"]),
    ("max_worker_prs_per_loop", len(supervisor.get("runs", [])), limits["max_worker_prs_per_loop"], ["state/supervisor-runs.json"]),
    ("max_stuck_items_per_loop", stuck_items, limits["max_stuck_items_per_loop"], ["state/stuck-handovers.json"]),
    ("max_verifier_failures_per_loop", verifier_failures, limits["max_verifier_failures_per_loop"], ["state/verifications/"]),
    ("max_open_autospec_prs", open_prs, limits["max_open_autospec_prs"], ["state/verifications/"]),
    ("max_active_issues", active_issues, limits["max_active_issues"], ["state/published-issues.json"]),
]
budgets = [{"budget": name, "used": used, "limit": limit, "status": status(used, limit), "evidence": evidence} for name, used, limit, evidence in rows]
overall = "exhausted" if any(row["status"] == "exhausted" for row in budgets) else ("near_limit" if any(row["status"] == "near_limit" for row in budgets) else "ok")
report = {"version": 1, "overall_status": overall, "budgets": budgets}
write_json(os.path.join(reports, "autonomy-budget.json"), report)
md = ["# Autonomy Budget", "", f"Overall status: **{overall}**", "", "| Budget | Used | Limit | Status | Evidence |", "| --- | ---: | ---: | --- | --- |"]
for row in budgets:
    md.append(f"| {row['budget']} | {row['used']} | {row['limit']} | {row['status']} | {', '.join(row['evidence'])} |")
write_text(os.path.join(reports, "autonomy-budget.md"), "\n".join(md))
print("autonomy budget: " + overall.upper())
sys.exit(1 if overall == "exhausted" else 0)
PY

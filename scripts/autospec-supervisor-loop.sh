#!/usr/bin/env bash
# scripts/autospec-supervisor-loop.sh — local operator-invoked multi-cycle supervisor.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-supervisor-loop.sh [--repo-root DIR] [--dry-run|--confirm] --max-cycles N [--issue NUMBER] [--repo OWNER/REPO]
EOF
}

die() {
    printf 'autospec-supervisor-loop: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
CONFIRM=0
MAX_CYCLES=1
ISSUE=""
GH_REPO=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        --max-cycles) MAX_CYCLES="$2"; shift 2 ;;
        --issue) ISSUE="$2"; shift 2 ;;
        --repo) GH_REPO="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$SCRIPT_DIR" "$CONFIRM" "$MAX_CYCLES" "$ISSUE" "$GH_REPO" "$*" <<'PY'
import json, os, subprocess, sys, uuid
from datetime import datetime, timezone

root, script_dir, confirm, max_cycles, issue, gh_repo = os.path.realpath(sys.argv[1]), os.path.realpath(sys.argv[2]), sys.argv[3] == "1", int(sys.argv[4]), sys.argv[5], sys.argv[6]
reports = os.path.join(root, ".autospec", "reports")
state = os.path.join(root, ".autospec", "state")
lock_path = os.path.join(root, ".autospec", "run.lock")
current_run_path = os.path.join(state, "current-run.json")
history_path = os.path.join(state, "run-history.json")
loop_runs_path = os.path.join(state, "supervisor-loop-runs.json")
stop_flag = os.path.expanduser("~/.autospec/stop.flag")
HARD_CAP = 10

def now():
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
def load(path, default):
    try:
        with open(path, "r", encoding="utf-8") as fh: return json.load(fh)
    except Exception: return default
def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh: json.dump(data, fh, indent=2, sort_keys=True); fh.write("\n")
def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh: fh.write(text.rstrip() + "\n")
def is_pid_alive(pid):
    try:
        os.kill(int(pid), 0)
        return True
    except Exception:
        return False
def run_script(args):
    cp = subprocess.run(args, cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return {"command": " ".join(args), "exit_code": cp.returncode, "stdout": cp.stdout.strip(), "stderr": cp.stderr.strip()}
def eligible_exists():
    if issue:
        return True
    published = load(os.path.join(state, "published-issues.json"), {"issues": []})
    if any(item.get("state", "open") == "open" for item in published.get("issues", [])):
        return True
    plan = load(os.path.join(reports, "issue-plan.json"), {"issues": []})
    return bool(plan.get("issues"))
def write_plan():
    plan = {"version": 1, "mode": "confirm" if confirm else "dry_run", "max_cycles": max_cycles, "hard_cap": HARD_CAP, "planned_cycles": min(max_cycles, HARD_CAP), "operator_invoked_only": True, "github_actions": False, "cron": False, "background_daemon": False}
    write_json(os.path.join(reports, "supervisor-loop-plan.json"), plan)
    write_text(os.path.join(reports, "supervisor-loop-plan.md"), "\n".join(["# Autospec Supervisor Loop", "", "## Summary", f"- Mode: `{plan['mode']}`", f"- Cycles requested: {max_cycles}", f"- Planned cycles: {plan['planned_cycles']}", "- GitHub Actions: disabled", "- Cron/background automation: disabled"]))
def finalize(run, stop_reason, status, exit_code, commands=None, lock_note=None):
    run.update({"status": status, "completed_at": now(), "stop_reason": stop_reason})
    write_json(current_run_path, run)
    history = load(history_path, {"schema": 1, "runs": []})
    history.setdefault("schema", 1); history.setdefault("runs", []).append(run); write_json(history_path, history)
    loops = load(loop_runs_path, {"schema": 1, "runs": []})
    loops.setdefault("schema", 1); loops.setdefault("runs", []).append(run); write_json(loop_runs_path, loops)
    result = {"version": 1, "mode": run["mode"], "run_id": run["run_id"], "max_cycles": max_cycles, "completed_cycles": run["completed_cycles"], "stop_reason": stop_reason, "issues_processed": run["issues_processed"], "prs_created_or_updated": [], "verifier_results": run["verifier_results"], "stuck_guidance_items": run["stuck_guidance_items"], "budget_usage": load(os.path.join(reports, "autonomy-budget.json"), {}), "commands": commands or [], "lock_note": lock_note}
    write_json(os.path.join(reports, "supervisor-loop-result.json"), result)
    md = ["# Autospec Supervisor Loop", "", "## Summary", f"- Stop reason: `{stop_reason}`", f"- Cycles completed: {run['completed_cycles']}", f"- Lock: {lock_note or 'released' if confirm else 'not required for dry-run'}", "", "## Cycles requested", str(max_cycles), "", "## Cycles completed", str(run["completed_cycles"]), "", "## Stop reason", stop_reason, "", "## Issues processed", ", ".join(map(str, run["issues_processed"])) or "None.", "", "## PRs created/updated", "None.", "", "## Verifier results", ", ".join(run["verifier_results"]) or "None.", "", "## Stuck/guidance items", ", ".join(run["stuck_guidance_items"]) or "None.", "", "## Budget usage", load(os.path.join(reports, "autonomy-budget.json"), {}).get("overall_status", "unknown"), "", "## Next recommended command", "`bash scripts/autospec-autonomy-status.sh`"]
    if lock_note and "stale lock" in lock_note:
        md += ["", "Recovery: inspect `.autospec/run.lock`; remove it only after confirming no loop is active."]
    write_text(os.path.join(reports, "supervisor-loop-result.md"), "\n".join(md))
    return exit_code
def acquire_lock():
    if not confirm:
        return True, "not required for dry-run"
    if os.path.isdir(lock_path):
        pid_file = os.path.join(lock_path, "pid")
        pid = open(pid_file).read().strip() if os.path.exists(pid_file) else ""
        if pid and is_pid_alive(pid):
            return False, "repo_lock_unavailable"
        return False, "stale lock detected; recovery instructions required"
    os.makedirs(lock_path, exist_ok=False)
    with open(os.path.join(lock_path, "pid"), "w", encoding="utf-8") as fh:
        fh.write(str(os.getpid()) + "\n")
    return True, "acquired"
def release_lock():
    if confirm and os.path.isdir(lock_path):
        pid_file = os.path.join(lock_path, "pid")
        pid = open(pid_file).read().strip() if os.path.exists(pid_file) else ""
        if pid == str(os.getpid()):
            try:
                os.remove(pid_file); os.rmdir(lock_path)
            except OSError:
                pass

write_plan()
if max_cycles < 1:
    raise SystemExit("max cycles must be >= 1")
if max_cycles > HARD_CAP:
    max_cycles = HARD_CAP

run = {"schema": 1, "run_id": str(uuid.uuid4()), "mode": "confirm" if confirm else "dry-run", "started_at": now(), "status": "running", "command": "autospec-supervisor-loop", "max_cycles": max_cycles, "completed_cycles": 0, "current_issue": issue or None, "stop_reason": None, "lock_path": ".autospec/run.lock", "issues_processed": [], "verifier_results": [], "stuck_guidance_items": []}
write_json(current_run_path, run)

ok, lock_note = acquire_lock()
if not ok:
    code = finalize(run, "repo_lock_unavailable", "failed", 1, lock_note=lock_note)
    print("supervisor loop: BLOCKED")
    sys.exit(code)

try:
    if os.path.exists(stop_flag):
        code = finalize(run, "stop_flag", "stopped", 1, lock_note=lock_note)
        print("supervisor loop: STOPPED")
        sys.exit(code)
    if not eligible_exists():
        code = finalize(run, "no_eligible_issues", "completed", 0, lock_note=lock_note)
        print("supervisor loop: NO ELIGIBLE ISSUES")
        sys.exit(code)

    commands = []
    for index in range(max_cycles):
        if os.path.exists(stop_flag):
            code = finalize(run, "stop_flag", "stopped", 1, commands, lock_note)
            print("supervisor loop: STOPPED")
            sys.exit(code)
        repeated = run_script(["bash", os.path.join(script_dir, "autospec-repeated-failures.sh"), "--repo-root", root, "--threshold", "2"]); commands.append(repeated)
        if repeated["exit_code"] != 0:
            code = finalize(run, "repeated_failure", "stopped", 1, commands, lock_note)
            print("supervisor loop: REPEATED FAILURE")
            sys.exit(code)
        budget = run_script(["bash", os.path.join(script_dir, "autospec-autonomy-budget.sh"), "--repo-root", root]); commands.append(budget)
        if budget["exit_code"] != 0:
            code = finalize(run, "budget_exhausted", "stopped", 1, commands, lock_note)
            print("supervisor loop: BUDGET EXHAUSTED")
            sys.exit(code)
        handover = load(os.path.join(state, "stuck-handovers.json"), {"handovers": []})
        if any(item.get("state") in {"needs-guidance", "stuck"} for item in handover.get("handovers", [])):
            code = finalize(run, "needs_guidance", "stopped", 1, commands, lock_note)
            print("supervisor loop: NEEDS GUIDANCE")
            sys.exit(code)
        cycle_cmd = ["bash", os.path.join(script_dir, "autospec-supervisor-cycle.sh"), "--repo-root", root, "--confirm" if confirm else "--dry-run", "--issue", issue or "1"]
        if gh_repo:
            cycle_cmd += ["--repo", gh_repo]
        cycle = run_script(cycle_cmd); commands.append(cycle)
        if cycle["exit_code"] != 0:
            code = finalize(run, "unsafe_state", "failed", 1, commands, lock_note)
            print("supervisor loop: UNSAFE")
            sys.exit(code)
        run["completed_cycles"] += 1
        run["issues_processed"].append(issue or "1")
        result = load(os.path.join(reports, "supervisor-cycle-result.json"), {})
        if result.get("verifier_verdict"):
            run["verifier_results"].append(result["verifier_verdict"])
    code = finalize(run, "completed_requested_cycles", "completed", 0, commands, lock_note)
    print("supervisor loop: PASS")
    sys.exit(code)
finally:
    release_lock()
PY

#!/usr/bin/env bash
# scripts/autospec-recovery-status.sh — report local recovery state.

set -eu
usage() { echo "Usage: autospec-recovery-status.sh [--repo-root DIR]"; }
die() { printf 'autospec-recovery-status: %s\n' "$*" >&2; exit 2; }
REPO_ROOT="$(pwd)"
while [ "$#" -gt 0 ]; do
    case "$1" in --repo-root) REPO_ROOT="$2"; shift 2 ;; -h|--help) usage; exit 0 ;; *) die "unknown arg: $1" ;; esac
done
[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" <<'PY'
import json, os, subprocess, sys, time
root = os.path.realpath(sys.argv[1]); autospec = os.path.join(root, ".autospec"); reports = os.path.join(autospec, "reports"); state = os.path.join(autospec, "state")
os.makedirs(reports, exist_ok=True)
def load(path, default):
    try: return json.load(open(path, encoding="utf-8"))
    except Exception: return default
lock = os.path.join(autospec, "run.lock")
current = load(os.path.join(state, "current-run.json"), {})
handover = load(os.path.join(state, "stuck-handovers.json"), {"handovers": []})
cp = subprocess.run(["git", "status", "--porcelain"], cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
generated_dirty = [line[3:] for line in cp.stdout.splitlines() if len(line) > 3 and line[3:].startswith(".autospec/")] if cp.returncode == 0 else []
report = {"schema": 1, "active_lock": os.path.exists(lock), "current_run": current, "stuck_handovers_requiring_sync": [h for h in handover.get("handovers", []) if h.get("state") in {"stuck", "needs-guidance"}], "uncommitted_generated_files": generated_dirty, "next_safe_recovery_command": "bash scripts/autospec-autonomy-status.sh"}
json.dump(report, open(os.path.join(reports, "recovery-status.json"), "w", encoding="utf-8"), indent=2, sort_keys=True); open(os.path.join(reports, "recovery-status.json"), "a").write("\n")
open(os.path.join(reports, "recovery-status.md"), "w", encoding="utf-8").write("\n".join(["# Autospec Recovery Status", "", f"- Active/stale lock: `{report['active_lock']}`", f"- Current run status: `{current.get('status', 'unknown')}`", f"- Stuck handovers requiring sync: {len(report['stuck_handovers_requiring_sync'])}", f"- Uncommitted generated files: {len(generated_dirty)}", "", "## Next safe recovery command", "", f"`{report['next_safe_recovery_command']}`", ""]))
print("recovery status: wrote reports")
PY

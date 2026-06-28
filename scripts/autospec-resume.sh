#!/usr/bin/env bash
# scripts/autospec-resume.sh — local resume helper for operator-invoked autonomy.

set -eu

REPO_ROOT="$(pwd)"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) REPO_ROOT="$2"; shift 2 ;;
        -h|--help) echo "Usage: autospec-resume.sh [--repo-root DIR]"; exit 0 ;;
        *) printf 'autospec-resume: unknown arg: %s\n' "$1" >&2; exit 2 ;;
    esac
done

REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"
FLAG_FILE="${HOME}/.autospec/stop.flag"

python3 - "$REPO_ROOT" "$FLAG_FILE" <<'PY'
import json, os, sys
root, flag = os.path.realpath(sys.argv[1]), sys.argv[2]
reports = os.path.join(root, ".autospec", "reports")
existed = os.path.exists(flag)
if existed:
    os.remove(flag)
report = {"version": 1, "stop_flag_path": flag, "stop_flag_existed": existed, "resume_performed": True, "auto_started_work": False, "next_recommended_command": "bash scripts/autospec-autonomy-status.sh"}
os.makedirs(reports, exist_ok=True)
with open(os.path.join(reports, "stop-status.json"), "w", encoding="utf-8") as fh:
    json.dump(report, fh, indent=2, sort_keys=True); fh.write("\n")
md = "\n".join(["# Stop / Resume Status", "", f"Stop flag existed: **{str(existed).lower()}**", "", "Resume removed the stop flag only. It does not start work.", "", "## Next", "`bash scripts/autospec-autonomy-status.sh`"])
with open(os.path.join(reports, "stop-status.md"), "w", encoding="utf-8") as fh:
    fh.write(md + "\n")
print("autospec resume: PASS")
PY

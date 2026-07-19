#!/usr/bin/env bash
# scripts/autospec-guide-issue.sh — post structured operator guidance to stuck issues.

set -eu

REPO_ROOT="$(pwd)"
CONFIRM=0
STUCK=""
SOURCE=""
MESSAGE_FILE=""
RESUME=0
GH_REPO=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) REPO_ROOT="$2"; shift 2 ;;
        --dry-run) CONFIRM=0; shift ;;
        --confirm) CONFIRM=1; shift ;;
        --stuck) STUCK="$2"; shift 2 ;;
        --source-issue) SOURCE="$2"; shift 2 ;;
        --message-file) MESSAGE_FILE="$2"; shift 2 ;;
        --resume) RESUME=1; shift ;;
        --repo) GH_REPO="$2"; shift 2 ;;
        -h|--help) echo "Usage: autospec-guide-issue.sh [--repo-root DIR] [--dry-run|--confirm] (--stuck N|--source-issue N) --message-file FILE [--resume]"; exit 0 ;;
        *) printf 'autospec-guide-issue: unknown arg: %s\n' "$1" >&2; exit 2 ;;
    esac
done

[ -n "$MESSAGE_FILE" ] || { echo "autospec-guide-issue: --message-file required" >&2; exit 2; }
[ -f "$MESSAGE_FILE" ] || { echo "autospec-guide-issue: message file missing: $MESSAGE_FILE" >&2; exit 2; }
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$CONFIRM" "$STUCK" "$SOURCE" "$MESSAGE_FILE" "$RESUME" "$GH_REPO" <<'PY'
import json, os, subprocess, sys, tempfile
root, confirm, stuck, source, msg_file, resume, repo = os.path.realpath(sys.argv[1]), sys.argv[2] == "1", sys.argv[3], sys.argv[4], sys.argv[5], sys.argv[6] == "1", sys.argv[7]
reports = os.path.join(root, ".autospec", "reports")
ledger_path = os.path.join(root, ".autospec", "state", "stuck-handovers.json")

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
def gh(args):
    cmd = ["gh"] + (["--repo", repo] if repo else []) + args
    cp = subprocess.run(cmd, cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if cp.returncode != 0: raise RuntimeError(cp.stderr.strip() or cp.stdout.strip())

message = open(msg_file, encoding="utf-8").read().strip()
body = "\n".join(["## Autospec operator guidance", "", message, "", "## Resume criteria", "- Guidance has been applied to the stuck issue.", "- `autospec:guidance-provided` is present.", "- `autospec:resume` is present." if resume else "- Resume label was not requested in this guidance."])
target = stuck or source
labels = ["autospec:guidance-provided"] + (["autospec:resume"] if resume else [])
actions = []
if confirm:
    fd, path = tempfile.mkstemp(prefix="autospec-guidance-", suffix=".md")
    with os.fdopen(fd, "w", encoding="utf-8") as fh: fh.write(body + "\n")
    gh(["issue", "comment", target, "--body-file", path]); actions.append("comment")
    for label in labels:
        gh(["issue", "edit", target, "--add-label", label]); actions.append("add " + label)
    ledger = load(ledger_path, {"schema": 1, "handovers": []})
    for item in ledger.get("handovers", []):
        if str(item.get("stuck_issue_number")) == str(target) or str(item.get("source_issue_number")) == str(source):
            item["guidance_detected"] = True
            item["resume_label_detected"] = resume
            item["state"] = "ready-to-resume" if resume else "guidance-provided"
    write_json(ledger_path, ledger)
report = {"version": 1, "mode": "confirm" if confirm else "dry_run", "target_issue": target, "labels_to_add": labels, "body": body, "actions": actions, "automatic_resume": False}
write_json(os.path.join(reports, "guidance-post-result.json" if confirm else "guidance-post-plan.json"), report)
write_text(os.path.join(reports, "guidance-post-result.md" if confirm else "guidance-post-plan.md"), "# Guidance Post\n\n" + body + "\n\nLabels: " + ", ".join(labels) + "\n\nNo automatic resume is performed.")
print("guidance post: PASS")
PY

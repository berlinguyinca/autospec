#!/usr/bin/env bash
# scripts/autospec-preflight.sh — local environment diagnostics for Autospec flows.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-preflight.sh [--repo-root DIR] [--dry-run] [--github]

Dry-run/local checks are default. GitHub CLI/auth is optional unless --github is
provided.
EOF
}

die() { printf 'autospec-preflight: %s\n' "$*" >&2; exit 2; }

REPO_ROOT="$(pwd)"
CHECK_GITHUB=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --dry-run) shift ;;
        --github) CHECK_GITHUB=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done
[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
python3 - "$REPO_ROOT" "$CHECK_GITHUB" "$SCRIPT_DIR" <<'PY'
import json
import os
import shutil
import subprocess
import sys

root = os.path.realpath(sys.argv[1])
check_github = sys.argv[2] == "1"
script_dir = os.path.realpath(sys.argv[3])
reports = os.path.join(root, ".autospec", "reports")
state = os.path.join(root, ".autospec", "state")

def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
        fh.write("\n")

def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text.rstrip() + "\n")

def check(name, ok, status_ok="pass", fix=""):
    return {"name": name, "status": status_ok if ok else "fail", "ok": bool(ok), "fix": fix}

def writable(path):
    os.makedirs(path, exist_ok=True)
    return os.access(path, os.W_OK)

gh = shutil.which("gh")
checks = [
    check("shell availability", bool(os.environ.get("SHELL")) or shutil.which("bash"), fix="Install bash or run from a POSIX shell."),
    check("git availability", bool(shutil.which("git")), fix="Install git."),
    check("repo root detection", os.path.isdir(root), fix="Pass --repo-root DIR."),
    check("required helper: autospec-start.sh", os.path.isfile(os.path.join(script_dir, "autospec-start.sh")), fix="Restore scripts/autospec-start.sh."),
    check("required helper: autospec-mvp-status.sh", os.path.isfile(os.path.join(script_dir, "autospec-mvp-status.sh")), fix="Restore scripts/autospec-mvp-status.sh."),
    {"name": "GitHub CLI availability", "status": "pass" if gh else ("fail" if check_github else "warn"), "ok": bool(gh), "fix": "Install gh only for confirmed GitHub flows." if not gh else ""},
    check("write access to .autospec/state", writable(state), fix="Ensure .autospec/state is writable."),
    check("write access to .autospec/reports", writable(reports), fix="Ensure .autospec/reports is writable."),
    {"name": "stop flag status", "status": "warn" if os.path.exists(os.path.expanduser("~/.autospec/stop.flag")) else "pass", "ok": not os.path.exists(os.path.expanduser("~/.autospec/stop.flag")), "fix": "Run scripts/autospec-resume.sh when ready."},
    {"name": "repo lock status", "status": "warn" if os.path.exists(os.path.join(root, ".autospec", "run.lock")) else "pass", "ok": not os.path.exists(os.path.join(root, ".autospec", "run.lock")), "fix": "Run scripts/autospec-recovery-status.sh for lock recovery."},
]
if check_github and gh:
    cp = subprocess.run(["gh", "auth", "status"], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    checks.append({"name": "GitHub auth status", "status": "pass" if cp.returncode == 0 else "fail", "ok": cp.returncode == 0, "fix": "Run gh auth login."})
required_fail = [c for c in checks if c["status"] == "fail"]
warnings = [c for c in checks if c["status"] == "warn"]
verdict = "needs_fixes" if required_fail else "pass_with_warnings" if warnings else "pass"
report = {"schema": 1, "verdict": verdict, "checks": checks, "github_required": check_github}
write_json(os.path.join(reports, "preflight.json"), report)
rows = "\n".join(f"| {c['name']} | {c['status']} | {c.get('fix','')} |" for c in checks)
write_text(os.path.join(reports, "preflight.md"), "\n".join([
    "# Autospec Preflight",
    "",
    f"## Verdict\n\n**{verdict}**",
    "",
    "## Checks",
    "",
    "| Check | Status | Fix |",
    "| --- | --- | --- |",
    rows,
    "",
    "## Notes",
    "",
    "- GitHub CLI is optional for purely local dry-run flows.",
]))
print(f"preflight: {verdict}")
sys.exit(0 if verdict in {"pass", "pass_with_warnings", "needs_fixes"} else 1)
PY

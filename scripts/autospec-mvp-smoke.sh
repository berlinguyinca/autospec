#!/usr/bin/env bash
# scripts/autospec-mvp-smoke.sh — safe local MVP release-candidate smoke test.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-mvp-smoke.sh [--repo-root DIR] [--dry-run] [--fixtures] [--repo DIR]

Runs local release-candidate checks only. No GitHub writes.
EOF
}

die() { printf 'autospec-mvp-smoke: %s\n' "$*" >&2; exit 2; }
REPO_ROOT="$(pwd)"
FIXTURES=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root|--repo) [ "$#" -ge 2 ] || die "$1 requires a value"; REPO_ROOT="$2"; shift 2 ;;
        --dry-run) shift ;;
        --fixtures) FIXTURES=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done
[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"

python3 - "$REPO_ROOT" "$SCRIPT_DIR" "$FIXTURES" <<'PY'
import json
import os
import subprocess
import sys

root, script_dir = os.path.realpath(sys.argv[1]), os.path.realpath(sys.argv[2])
reports = os.path.join(root, ".autospec", "reports")
os.makedirs(reports, exist_ok=True)

def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True); fh.write("\n")

def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text.rstrip() + "\n")

def run(label, args, required=False):
    cp = subprocess.run(args, cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return {"label": label, "command": " ".join(args), "exit_code": cp.returncode, "required": required, "stdout": cp.stdout[-2000:], "stderr": cp.stderr[-2000:]}

commands = [
    ("preflight", [os.path.join(script_dir, "autospec-preflight.sh"), "--repo-root", root, "--dry-run"], True),
    ("command audit", [os.path.join(script_dir, "autospec-command-audit.sh"), "--repo-root", root], False),
    ("state validation", [os.path.join(script_dir, "autospec-validate-state.sh"), "--repo-root", root], False),
    ("sensitive output audit", [os.path.join(script_dir, "autospec-sensitive-output-audit.sh"), "--repo-root", root], False),
    ("report quality", [os.path.join(script_dir, "autospec-report-quality.sh"), "--repo-root", root], False),
    ("check type coverage", [os.path.join(script_dir, "autospec-check-type-coverage.sh"), "--repo-root", root], False),
    ("template coverage", [os.path.join(script_dir, "autospec-template-coverage.sh"), "--repo-root", root], False),
    ("audit to backlog dry-run", [os.path.join(script_dir, "autospec-audit-to-backlog.sh"), "--repo-root", root, "--dry-run"], False),
    ("autonomy status", [os.path.join(script_dir, "autospec-autonomy-status.sh"), "--repo-root", root], False),
    ("mvp status", [os.path.join(script_dir, "autospec-mvp-status.sh"), "--repo-root", root], False),
    ("report index", [os.path.join(script_dir, "autospec-report-index.sh"), "--repo-root", root], False),
]
results = [run(label, ["bash"] + args, required) for label, args, required in commands if os.path.exists(args[0])]
failures = [r for r in results if r["exit_code"] != 0 and r["required"]]
warnings = [r for r in results if r["exit_code"] != 0 and not r["required"]]
verdict = "blocked" if failures else "pass_with_warnings" if warnings else "pass"
generated_reports = sorted(f".autospec/reports/{name}" for name in os.listdir(reports) if name.endswith((".md", ".json")))
report = {
    "schema": 1,
    "verdict": verdict,
    "commands_run": results,
    "generated_reports": generated_reports,
    "safety_checks": {
        "github_writes": False,
        "scheduler_added": False,
        "auto_merge": False,
        "self_approval": False,
    },
    "required_fixes_before_release": [r["label"] for r in failures],
    "recommended_next_command": "bash scripts/autospec-mvp-status.sh" if verdict != "blocked" else "fix required smoke failures",
}
write_json(os.path.join(reports, "mvp-smoke.json"), report)
rows = "\n".join(f"| {r['label']} | {r['exit_code']} | `{r['command']}` |" for r in results)
write_text(os.path.join(reports, "mvp-smoke.md"), "\n".join([
    "# Autospec MVP Smoke Report",
    "",
    "## Verdict",
    "",
    f"**{verdict}**",
    "",
    "## Commands run",
    "",
    "| Check | Exit | Command |",
    "| --- | ---: | --- |",
    rows,
    "",
    "## Pass/fail summary",
    "",
    f"- Required failures: {len(failures)}",
    f"- Warnings: {len(warnings)}",
    "",
    "## Generated reports",
    "",
    "\n".join(f"- `{item}`" for item in generated_reports[:40]) or "- None.",
    "",
    "## Policy source status",
    "",
    "- See `preflight.md`, `policy-source-validation.md`, and `policy-compatibility.md` when present.",
    "",
    "## Digital Twin status",
    "",
    "- See `digital-twin.md` when present.",
    "",
    "## Rule audit status",
    "",
    "- See `constitution-audit.md` and `rule-check-results.md` when present.",
    "",
    "## Backlog status",
    "",
    "- See `audit-to-backlog-result.md` and `github-issue-publish-plan-v3.md` when present.",
    "",
    "## Worker/verifier/supervisor readiness",
    "",
    "- Local commands are present; no worker was executed by smoke.",
    "",
    "## Safety checks",
    "",
    "- GitHub writes: false",
    "- Scheduler added: false",
    "- Auto-merge: false",
    "- Self-approval: false",
    "",
    "## Known limitations",
    "",
    "- See `docs/KNOWN_LIMITATIONS.md`.",
    "",
    "## Required fixes before release",
    "",
    "\n".join(f"- {item}" for item in report["required_fixes_before_release"]) or "- None.",
    "",
    "## Recommended next command",
    "",
    f"`{report['recommended_next_command']}`",
]))
print(f"mvp smoke: {verdict}")
sys.exit(0 if verdict in {"pass", "pass_with_warnings"} else 1)
PY

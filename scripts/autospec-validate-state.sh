#!/usr/bin/env bash
# scripts/autospec-validate-state.sh — validate generated Autospec state/report artifacts.

set -eu
usage() { echo "Usage: autospec-validate-state.sh [--repo-root DIR]"; }
die() { printf 'autospec-validate-state: %s\n' "$*" >&2; exit 2; }
REPO_ROOT="$(pwd)"
while [ "$#" -gt 0 ]; do
    case "$1" in --repo-root) REPO_ROOT="$2"; shift 2 ;; -h|--help) usage; exit 0 ;; *) die "unknown arg: $1" ;; esac
done
[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" <<'PY'
import json, os, re, sys
root = os.path.realpath(sys.argv[1]); reports = os.path.join(root, ".autospec", "reports"); state = os.path.join(root, ".autospec", "state")
os.makedirs(reports, exist_ok=True)
findings = []
for folder in [reports, state]:
    if not os.path.isdir(folder): continue
    for dirpath, _, files in os.walk(folder):
        for name in files:
            path = os.path.join(dirpath, name); rel = os.path.relpath(path, root)
            if name.endswith(".json"):
                try:
                    data = json.load(open(path, encoding="utf-8"))
                    if isinstance(data, dict) and not any(k in data for k in ["schema", "version"]):
                        findings.append({"severity": "warn", "file": rel, "summary": "JSON lacks schema/version"})
                except Exception as exc:
                    findings.append({"severity": "fail", "file": rel, "summary": f"invalid JSON: {exc}"})
            try:
                text = open(path, encoding="utf-8", errors="ignore").read()
            except OSError:
                continue
            if re.search(r"gh[pousr]_[A-Za-z0-9_]{20,}|-----BEGIN [A-Z ]*PRIVATE KEY-----", text):
                findings.append({"severity": "fail", "file": rel, "summary": "possible secret in generated artifact"})
            if root in text:
                findings.append({"severity": "warn", "file": rel, "summary": "absolute local path appears in artifact"})
status = "fail" if any(f["severity"] == "fail" for f in findings) else "warn" if findings else "pass"
report = {"schema": 1, "status": status, "findings": findings, "required_state": [".autospec/state", ".autospec/reports"]}
json.dump(report, open(os.path.join(reports, "state-validation.json"), "w", encoding="utf-8"), indent=2, sort_keys=True); open(os.path.join(reports, "state-validation.json"), "a").write("\n")
rows = "\n".join(f"| {f['severity']} | `{f['file']}` | {f['summary']} |" for f in findings)
open(os.path.join(reports, "state-validation.md"), "w", encoding="utf-8").write("\n".join(["# Autospec State Validation", "", f"## Status\n\n**{status}**", "", "| Severity | File | Summary |", "| --- | --- | --- |", rows or "| pass | none | no findings |", ""]))
print(f"state validation: {status}")
sys.exit(0 if status in {"pass", "warn"} else 1)
PY

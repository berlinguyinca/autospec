#!/usr/bin/env bash
# scripts/autospec-sensitive-output-audit.sh — scan generated Autospec outputs for leaked secrets.

set -eu
usage() { echo "Usage: autospec-sensitive-output-audit.sh [--repo-root DIR]"; }
die() { printf 'autospec-sensitive-output-audit: %s\n' "$*" >&2; exit 2; }
REPO_ROOT="$(pwd)"
while [ "$#" -gt 0 ]; do
    case "$1" in --repo-root) REPO_ROOT="$2"; shift 2 ;; -h|--help) usage; exit 0 ;; *) die "unknown arg: $1" ;; esac
done
[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" <<'PY'
import json, os, re, sys
root = os.path.realpath(sys.argv[1]); base = os.path.join(root, ".autospec"); reports = os.path.join(base, "reports")
os.makedirs(reports, exist_ok=True)
patterns = [
    ("github_token", re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}")),
    ("private_key", re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")),
    ("authorization_header", re.compile(r"Authorization:\s*Bearer\s+\S+", re.I)),
    ("password_assignment", re.compile(r"\b(password|secret|token)\s*=\s*['\"]?[^'\"\s]{8,}", re.I)),
    ("database_url_credentials", re.compile(r"\b\w+://[^/\s:@]+:[^@\s]+@")),
]
findings = []
for sub in ["reports", "state"]:
    folder = os.path.join(base, sub)
    if not os.path.isdir(folder):
        continue
    for dirpath, _, files in os.walk(folder):
        for name in files:
            path = os.path.join(dirpath, name)
            rel = os.path.relpath(path, root)
            try:
                text = open(path, encoding="utf-8", errors="ignore").read()
            except OSError:
                continue
            for kind, rx in patterns:
                for match in rx.finditer(text):
                    findings.append({"file": rel, "type": kind, "match": "[REDACTED]", "suggestion": "Remove or redact this generated output before publishing."})
status = "fail" if findings else "pass"
report = {"schema": 1, "status": status, "findings": findings}
json.dump(report, open(os.path.join(reports, "sensitive-output-audit.json"), "w", encoding="utf-8"), indent=2, sort_keys=True); open(os.path.join(reports, "sensitive-output-audit.json"), "a").write("\n")
rows = "\n".join(f"| `{f['file']}` | {f['type']} | {f['match']} | {f['suggestion']} |" for f in findings)
open(os.path.join(reports, "sensitive-output-audit.md"), "w", encoding="utf-8").write("\n".join(["# Autospec Sensitive Output Audit", "", f"## Status\n\n**{status}**", "", "| File | Type | Match | Remediation |", "| --- | --- | --- | --- |", rows or "| none | none | none | none |", ""]))
print(f"sensitive output audit: {status}")
sys.exit(1 if findings else 0)
PY

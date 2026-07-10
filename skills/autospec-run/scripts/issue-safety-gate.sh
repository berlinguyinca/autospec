#!/usr/bin/env bash
# issue-safety-gate.sh — shared fail-closed autospec-run issue safety predicate.

AUTOSPEC_ISSUE_SAFETY_GATE_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

autospec_issue_safety_linter_path() {
    if [ -n "${AUTOSPEC_SCRIPTS_DIR:-}" ] && [ -f "$AUTOSPEC_SCRIPTS_DIR/lint-issue-safety.sh" ]; then
        printf '%s\n' "$AUTOSPEC_SCRIPTS_DIR/lint-issue-safety.sh"
        return 0
    fi
    if [ -f "$AUTOSPEC_ISSUE_SAFETY_GATE_DIR/lint-issue-safety.sh" ]; then
        printf '%s\n' "$AUTOSPEC_ISSUE_SAFETY_GATE_DIR/lint-issue-safety.sh"
        return 0
    fi
    if [ -f "$AUTOSPEC_ISSUE_SAFETY_GATE_DIR/../../../scripts/lint-issue-safety.sh" ]; then
        printf '%s\n' "$AUTOSPEC_ISSUE_SAFETY_GATE_DIR/../../../scripts/lint-issue-safety.sh"
        return 0
    fi
    if [ -f "$HOME/.autospec/scripts/lint-issue-safety.sh" ]; then
        printf '%s\n' "$HOME/.autospec/scripts/lint-issue-safety.sh"
        return 0
    fi
    return 1
}

autospec_issue_safety_gate_result() {
    issue_json="$(cat)"
    linter_path="$(autospec_issue_safety_linter_path || true)"
    ISSUE_JSON="$issue_json" AUTOSPEC_ISSUE_SAFETY_LINTER="$linter_path" python3 - <<'PY'
import json
import os
import subprocess
import sys
import tempfile

BEGIN = "<!-- autospec-safety:begin -->"
END = "<!-- autospec-safety:end -->"


def result(ok, detail):
    payload = {"ok": ok, "reason": detail}
    print(json.dumps(payload, separators=(",", ":")))


try:
    issue = json.loads(os.environ.get("ISSUE_JSON", ""))
except Exception:
    result(False, "invalid_issue_json")
    sys.exit(0)

labels = set()
for row in issue.get("labels") or []:
    if isinstance(row, dict) and row.get("name"):
        labels.add(str(row["name"]))
    elif isinstance(row, str):
        labels.add(row)

if "security:quarantined" in labels:
    result(False, "security_quarantined")
    sys.exit(0)
if "safety:reviewed" not in labels:
    result(False, "missing_safety_reviewed")
    sys.exit(0)

body = issue.get("body") or ""
begin_count = body.count(BEGIN)
end_count = body.count(END)
if begin_count != 1 or end_count != 1:
    result(False, "invalid_safety_markers")
    sys.exit(0)

begin = body.find(BEGIN)
end = body.find(END)
if begin < 0 or end < 0 or begin >= end:
    result(False, "invalid_safety_markers")
    sys.exit(0)

prefix = body[:begin]
previous_headings = [line.strip() for line in prefix.splitlines() if line.startswith("## ")]
if not previous_headings or previous_headings[-1] != "## Safety review":
    result(False, "missing_safety_review_heading")
    sys.exit(0)

block = body[begin + len(BEGIN):end]
decision_prefix = "- **decision:**"
decision_pass = "- **decision:** `SAFETY_PASS`"
decision_lines = [
    line for line in block.splitlines()
    if line.strip().startswith(decision_prefix)
]
if len(decision_lines) != 1:
    result(False, "missing_safety_pass")
    sys.exit(0)
if decision_lines[0] != decision_pass:
    result(False, "non_pass_safety_decision")
    sys.exit(0)

linter = os.environ.get("AUTOSPEC_ISSUE_SAFETY_LINTER") or ""
if not linter:
    result(False, "missing_safety_linter")
    sys.exit(0)

title = str(issue.get("title") or "")
author = issue.get("author") or {}
actor = author.get("login") if isinstance(author, dict) else ""
actor = str(actor or "")
body_without_review = body[:prefix.rfind("## Safety review")] + body[end + len(END):]
with tempfile.NamedTemporaryFile("w", encoding="utf-8", delete=False) as handle:
    handle.write(body_without_review)
    body_file = handle.name
try:
    cmd = ["bash", linter, "--json", "--title", title]
    if actor:
        cmd.extend(["--actor", actor])
    cmd.append(body_file)
    completed = subprocess.run(cmd, text=True, capture_output=True, check=False)
finally:
    try:
        os.unlink(body_file)
    except OSError:
        pass

if completed.returncode == 1:
    result(False, "current_body_safety_ambiguous")
    sys.exit(0)
if completed.returncode == 2:
    result(False, "current_body_safety_block")
    sys.exit(0)
if completed.returncode != 0:
    result(False, "current_body_safety_error")
    sys.exit(0)

result(True, "pass")
PY
}

autospec_issue_safety_gate_passes() {
    result="$(autospec_issue_safety_gate_result)"
    printf '%s\n' "$result" | jq -e '.ok == true' >/dev/null 2>&1
}

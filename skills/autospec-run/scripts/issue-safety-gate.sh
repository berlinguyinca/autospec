#!/usr/bin/env bash
# issue-safety-gate.sh — shared fail-closed autospec-run issue safety predicate.

autospec_issue_safety_gate_result() {
    issue_json="$(cat)"
    ISSUE_JSON="$issue_json" python3 - <<'PY'
import json
import os
import sys

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

result(True, "pass")
PY
}

autospec_issue_safety_gate_passes() {
    result="$(autospec_issue_safety_gate_result)"
    printf '%s\n' "$result" | jq -e '.ok == true' >/dev/null 2>&1
}

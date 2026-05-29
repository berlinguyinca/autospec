#!/usr/bin/env bash
# scripts/explore-research/open-issues.sh — deterministic researcher #4 of 4.
#
# Lists open GitHub issues excluding the autospec:v2-flow label and emits
# one proposal per issue, carrying the issue title + URL as citation.
#
# Cap: 50 issues per round.
#
# Output: JSON to stdout per the spec contract.

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
MAX_ISSUES=50

cd "$REPO_ROOT" || { echo '{"source":"open-issues","proposals":[]}'; exit 0; }

# In test mode, allow injection via AUTOSPEC_TEST_ISSUES_JSON (path to JSON file).
if [ -n "${AUTOSPEC_TEST_ISSUES_JSON:-}" ] && [ -f "$AUTOSPEC_TEST_ISSUES_JSON" ]; then
    RAW_JSON="$(cat "$AUTOSPEC_TEST_ISSUES_JSON")"
elif command -v gh >/dev/null 2>&1; then
    RAW_JSON="$(gh issue list --state open --limit "$MAX_ISSUES" \
        --json number,title,url,labels 2>/dev/null || echo '[]')"
else
    RAW_JSON='[]'
fi

export RAW_JSON
export MAX_ISSUES

python3 - <<'PY'
import json, os
cap = int(os.environ.get("MAX_ISSUES","50"))
try:
    issues = json.loads(os.environ.get("RAW_JSON","[]"))
except Exception:
    issues = []
proposals = []
for it in issues:
    if not isinstance(it, dict):
        continue
    labels = it.get("labels") or []
    label_names = []
    for l in labels:
        if isinstance(l, dict):
            label_names.append(l.get("name",""))
        elif isinstance(l, str):
            label_names.append(l)
    if "autospec:v2-flow" in label_names:
        continue
    title = (it.get("title") or "").strip()
    url   = it.get("url") or ""
    num   = it.get("number")
    if not title:
        continue
    proposals.append({
        "title": f"track: open issue #{num} — {title[:80]}",
        "evidence": f"Open GitHub issue #{num} ({url}): {title}",
        "estimated_complexity": "medium",
        "confidence": 0.65,
    })
    if len(proposals) >= cap:
        break
print(json.dumps({"source":"open-issues","proposals":proposals}))
PY

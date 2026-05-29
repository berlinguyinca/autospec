#!/usr/bin/env bash
# scripts/explore-research/prior-reports.sh — deterministic researcher #2 of 4.
#
# Reads .autospec/run-summary.md, .autospec/explore-summary.md, and the last
# 10 .autospec/runs/*/report.md. Uses the matchers from
# scripts/lib/extract-matchers.sh to harvest unconsumed Next-steps content
# and emits a proposal per harvested item.
#
# Cap: last 10 reports.
#
# Output: JSON to stdout per the spec contract.

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# Matchers live alongside this script (../lib/extract-matchers.sh), NOT in the
# target repo being explored. Allow override via env.
MATCHERS="${AUTOSPEC_MATCHERS_LIB:-$SCRIPT_DIR/../lib/extract-matchers.sh}"
SUMMARY_DIR="${AUTOSPEC_SUMMARY_DIR:-$REPO_ROOT/.autospec}"
MAX_REPORTS=10

cd "$REPO_ROOT" || { echo '{"source":"prior-reports","proposals":[]}'; exit 0; }

if [ ! -f "$MATCHERS" ]; then
    echo '{"source":"prior-reports","proposals":[]}'
    exit 0
fi

# shellcheck disable=SC1090
. "$MATCHERS"

# Build the list of input files.
inputs=()
[ -f "$SUMMARY_DIR/run-summary.md" ]     && inputs+=("$SUMMARY_DIR/run-summary.md")
[ -f "$SUMMARY_DIR/explore-summary.md" ] && inputs+=("$SUMMARY_DIR/explore-summary.md")
if [ -d "$SUMMARY_DIR/runs" ]; then
    # most recent N report.md files
    while IFS= read -r f; do
        inputs+=("$f")
    done < <(find "$SUMMARY_DIR/runs" -type f -name 'report.md' 2>/dev/null \
        | xargs -I{} stat -f '%m %N' {} 2>/dev/null \
        | sort -rn | head -n "$MAX_REPORTS" | awk '{$1=""; sub(/^ /,""); print}')
    # Fallback for GNU stat (Linux)
    if [ "${#inputs[@]}" -le 2 ]; then
        while IFS= read -r f; do
            inputs+=("$f")
        done < <(find "$SUMMARY_DIR/runs" -type f -name 'report.md' \
            -printf '%T@ %p\n' 2>/dev/null \
            | sort -rn | head -n "$MAX_REPORTS" | awk '{$1=""; sub(/^ /,""); print}')
    fi
fi

if [ "${#inputs[@]}" -eq 0 ]; then
    echo '{"source":"prior-reports","proposals":[]}'
    exit 0
fi

# Harvest from each input via all 4 matchers; collect into a Python aggregator.
harvest_tmp="$(mktemp -t prior-reports.XXXXXX)"
trap 'rm -f "$harvest_tmp"' EXIT

for f in "${inputs[@]}"; do
    [ -f "$f" ] || continue
    MSG="$(cat "$f" 2>/dev/null)"
    [ -z "$MSG" ] && continue
    {
        echo "===SOURCE: $f==="
        echo "---section-headings---"
        MSG="$MSG" extract_section_headings
        echo "---fenced-blocks---"
        MSG="$MSG" extract_fenced_blocks
        echo "---numbered-action---"
        MSG="$MSG" extract_numbered_action_list
        echo "---next-prefix---"
        MSG="$MSG" extract_next_prefix_continuations
        echo "---recommend---"
        MSG="$MSG" extract_recommend_sentences
    } >> "$harvest_tmp"
done

python3 - "$harvest_tmp" <<'PY'
import json, sys, re

path = sys.argv[1]
with open(path, 'r', encoding='utf-8', errors='ignore') as fh:
    text = fh.read()

proposals = []
current_source = "unknown"
seen = set()

# Split by SOURCE markers to track which report each item came from.
chunks = re.split(r'===SOURCE: (.+?)===', text)
# chunks[0] is whatever was before the first SOURCE (usually empty);
# then pairs of (source_path, body).
for i in range(1, len(chunks), 2):
    src = chunks[i].strip()
    body = chunks[i+1] if i+1 < len(chunks) else ""
    for line in body.splitlines():
        s = line.strip()
        # Skip section dividers and blanks and markdown headings.
        if not s or s.startswith('---') or s.startswith('##'):
            continue
        # Strip leading list/bullet/number markers.
        cleaned = re.sub(r'^\s*([0-9]+\.|[-*+])\s+', '', s)
        cleaned = cleaned.strip(' \t-*`')
        if len(cleaned) < 12:
            continue
        norm = cleaned.lower()[:80]
        if norm in seen:
            continue
        seen.add(norm)
        title_snip = cleaned[:80].replace('"', "'")
        proposals.append({
            "title": f"chore: follow up on prior report — {title_snip}",
            "evidence": f"Unconsumed Next-steps content from {src}: {cleaned[:240]}",
            "estimated_complexity": "small",
            "confidence": 0.70,
        })

print(json.dumps({"source": "prior-reports", "proposals": proposals}))
PY

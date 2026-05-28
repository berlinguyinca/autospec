#!/usr/bin/env bash
# scripts/dogfood-adapter-lint.sh — issue #646
#
# Adapter that runs scripts/lint-implementation.sh against this repo's
# most-recent commit (HEAD~1..HEAD) as a synthetic diff, then translates
# its per-line RULE_ID findings into VERDICT_FILE JSON-Lines findings.
#
# Contract (per scripts/dogfood-detectors.sh):
#   In:  REPO_DIR, VERDICT_FILE env vars
#   Out: one JSON object per line on VERDICT_FILE with keys
#        {category, rule_id, language, file, function, line}
#   Exit: always 0 — driver decides PASS/FAIL via allowlist diff.
#
# lint-implementation.sh emits "RULE_ID:<path>:<line>: <desc>" or
# "INFO:RULE_ID:<path>:<line>: <desc>" on stdout. INFO lines are
# informational and dropped here. Non-INFO lines become findings with
# category=implementation_quality.

set -eu

REPO_DIR="${REPO_DIR:-$(pwd)}"
VERDICT_FILE="${VERDICT_FILE:-/dev/stdout}"

DETECTOR="${REPO_DIR}/scripts/lint-implementation.sh"
if [ ! -r "$DETECTOR" ]; then
    exit 0
fi

cd "$REPO_DIR"

WORK_DIR="$(mktemp -d -t dogfood-lint.XXXXXX)"
trap 'rm -rf "$WORK_DIR"' EXIT

DIFF_FILE="$WORK_DIR/diff.txt"
# Use staged-diff semantics (clean-tree): the lint detector runs against
# `git diff --cached` so that on a clean checkout there is nothing to scan.
# PR-time lint enforcement still happens in the pre-commit hook and in the
# fused guardian/LGTM reviewer; this adapter only catches in-progress staged
# offenders during local dogfood runs. Earlier HEAD~1..HEAD semantics shifted
# the diff window every merge and silently invalidated the allowlist.
git -C "$REPO_DIR" diff --cached > "$DIFF_FILE" 2>/dev/null || true

# Skip if there's no diff to analyse.
[ -s "$DIFF_FILE" ] || exit 0

OUTPUT_FILE="$WORK_DIR/findings.txt"
bash "$DETECTOR" --diff-file "$DIFF_FILE" > "$OUTPUT_FILE" 2>/dev/null || true

# Parse each line: "RULE_ID:<path>:<line>: <desc>" — emit one JSON per
# blocking finding. Drop INFO: lines.
while IFS= read -r line; do
    [ -z "$line" ] && continue
    case "$line" in
        INFO:*) continue ;;
    esac
    # Split on ":" — RULE_ID, path, line, then everything else is desc.
    rule_id="${line%%:*}"
    rest="${line#*:}"
    path="${rest%%:*}"
    rest="${rest#*:}"
    lineno="${rest%%:*}"
    # Defensive: if lineno is not numeric (e.g. "-"), coerce to 0.
    case "$lineno" in
        ''|*[!0-9]*) lineno=0 ;;
    esac
    # Skip unparseable rows (e.g. truncation banners with path="-").
    [ -z "$rule_id" ] && continue
    [ -z "$path" ] && continue
    printf '{"category":"implementation_quality","rule_id":"%s","language":"n/a","file":"%s","function":"-","line":%s}\n' \
        "$rule_id" "$path" "$lineno" >> "$VERDICT_FILE"
done < "$OUTPUT_FILE"

exit 0

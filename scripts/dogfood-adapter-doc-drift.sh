#!/usr/bin/env bash
# scripts/dogfood-adapter-doc-drift.sh — issue #646
#
# Adapter that runs skills/autospec-shared/scripts/check-doc-drift.sh against
# this repo's most-recent commit (HEAD~1..HEAD) as a synthetic "PR", then
# translates the gate JSON into VERDICT_FILE JSON-Lines findings.
#
# Contract (per scripts/dogfood-detectors.sh):
#   In:  REPO_DIR, VERDICT_FILE env vars
#   Out: one JSON object per line on VERDICT_FILE with keys
#        {category, rule_id, language, file, function, line}
#   Exit: always 0 — driver decides PASS/FAIL via allowlist diff.
#
# The check-doc-drift gate emits a JSON object with `drift`, `missing_scope`,
# `visual_stale` arrays. Each entry becomes one finding line. Rule IDs:
#   DOC_DRIFT_HARD_FAIL  — `drift[]`
#   DOC_DRIFT_MISSING_SCOPE — `missing_scope[]`
#   DOC_DRIFT_VISUAL_STALE  — `visual_stale[]`
#   DOC_DRIFT_WARN          — `drift_warn[]`

set -eu

REPO_DIR="${REPO_DIR:-$(pwd)}"
VERDICT_FILE="${VERDICT_FILE:-/dev/stdout}"

DETECTOR="${REPO_DIR}/skills/autospec-shared/scripts/check-doc-drift.sh"
if [ ! -x "$DETECTOR" ] && [ ! -r "$DETECTOR" ]; then
    # Detector unavailable in this repo — emit no findings.
    exit 0
fi

cd "$REPO_DIR"

# Build a synthetic diff of the most-recent commit. If the repo has no
# prior commit, fall back to working-tree mode (no findings most likely).
WORK_DIR="$(mktemp -d -t dogfood-doc-drift.XXXXXX)"
trap 'rm -rf "$WORK_DIR"' EXIT

GATE_JSON="$WORK_DIR/gate.json"

# Use --working-tree (clean-tree semantics): the detector compares uncommitted
# changes vs HEAD. On a clean checkout this returns no findings, which is the
# correct dogfood signal — PR-time drift is checked at PR time, not against a
# moving allowlist on main. Earlier baseline-pin and HEAD~1 approaches both
# accumulated noise as main advanced (issue #651 + follow-up).
bash "$DETECTOR" --working-tree > "$GATE_JSON" 2>/dev/null || true

# If the gate produced no JSON (e.g. errored out) leave VERDICT_FILE alone.
[ -s "$GATE_JSON" ] || exit 0

emit() {
    # emit RULE_ID FILE FUNCTION LINE
    local rule_id="$1" file="$2" func="$3" line="$4"
    printf '{"category":"code_health:doc_drift","rule_id":"%s","language":"n/a","file":"%s","function":"%s","line":%s}\n' \
        "$rule_id" "$file" "$func" "$line" >> "$VERDICT_FILE"
}

if command -v jq >/dev/null 2>&1; then
    jq -r '.drift[]? | [.doc_file // "-", .heading // "-"] | @tsv' "$GATE_JSON" 2>/dev/null \
        | while IFS=$'\t' read -r doc heading; do
              [ -z "$doc" ] && continue
              emit "DOC_DRIFT_HARD_FAIL" "$doc" "$heading" 0
          done
    jq -r '.drift_warn[]? | [.doc_file // "-", .heading // "-"] | @tsv' "$GATE_JSON" 2>/dev/null \
        | while IFS=$'\t' read -r doc heading; do
              [ -z "$doc" ] && continue
              emit "DOC_DRIFT_WARN" "$doc" "$heading" 0
          done
    jq -r '.missing_scope[]? | .source_file // "-"' "$GATE_JSON" 2>/dev/null \
        | while IFS= read -r src; do
              [ -z "$src" ] || [ "$src" = "-" ] && continue
              emit "DOC_DRIFT_MISSING_SCOPE" "$src" "-" 0
          done
    jq -r '.visual_stale[]? | [.doc_file // "-", .screenshot // "-"] | @tsv' "$GATE_JSON" 2>/dev/null \
        | while IFS=$'\t' read -r doc shot; do
              [ -z "$doc" ] && continue
              emit "DOC_DRIFT_VISUAL_STALE" "$doc" "$shot" 0
          done
fi

exit 0

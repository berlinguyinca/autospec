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
#   DOC_FULL_AUDIT          — repo-wide completeness gap from `/autospec-doc --full`
#
# Sweep upgrade (spec §D6 row 3, issue #924): the docs-drift sweep check is no
# longer detect-only. It first invokes `/autospec-doc --full` — full regen plus
# the repo-wide completeness audit — via scripts/doc-orchestrator.mjs, then
# surfaces any missing parts the audit reports as findings (rule
# DOC_FULL_AUDIT). The detect-only drift gate below still runs afterward so
# stale/contradicting docs continue to surface alongside completeness gaps.

set -eu

REPO_DIR="${REPO_DIR:-$(pwd)}"
VERDICT_FILE="${VERDICT_FILE:-/dev/stdout}"

DETECTOR="${REPO_DIR}/skills/autospec-shared/scripts/check-doc-drift.sh"
if [ ! -x "$DETECTOR" ] && [ ! -r "$DETECTOR" ]; then
    # Detector unavailable in this repo — emit no findings.
    exit 0
fi

cd "$REPO_DIR"

emit() {
    # emit RULE_ID FILE FUNCTION LINE
    local rule_id="$1" file="$2" func="$3" line="$4"
    printf '{"category":"code_health:doc_drift","rule_id":"%s","language":"n/a","file":"%s","function":"%s","line":%s}\n' \
        "$rule_id" "$file" "$func" "$line" >> "$VERDICT_FILE"
}

# ── /autospec-doc --full (spec §D6 row 3) ─────────────────────────────────────
# Invoke the single doc engine in full mode: regenerate every audience and run
# the repo-wide completeness audit. The orchestrator exits 2 when the repo has
# no `documentation:` config (nothing to regenerate) — a graceful skip, not a
# finding. Any audit line the engine prints (`missing: <path>`) becomes one
# DOC_FULL_AUDIT finding so repo-wide gaps surface through the sweep's existing
# finding emission.
ORCHESTRATOR="${REPO_DIR}/skills/autospec-doc/scripts/doc-orchestrator.mjs"
if [ -r "$ORCHESTRATOR" ] && command -v node >/dev/null 2>&1; then
    FULL_OUT="$(node "$ORCHESTRATOR" --full 2>&1)"; full_rc=$?
    if [ "$full_rc" != "2" ]; then
        # rc 0 = regen+audit ran; rc !=0,!=2 = engine error (still surface gaps).
        printf '%s\n' "$FULL_OUT" \
            | sed -n 's/.*[Mm]issing:[[:space:]]*\([^[:space:]]*\).*/\1/p' \
            | while IFS= read -r missing; do
                  [ -n "$missing" ] && emit "DOC_FULL_AUDIT" "$missing" "-" 0
              done
    fi
fi

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

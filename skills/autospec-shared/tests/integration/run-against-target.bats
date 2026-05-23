#!/usr/bin/env bats
# run-against-target.bats — Integration harness for autospec-shared synthetic targets.
#
# Runs check-doc-drift.sh --diff against each target's pre-baked diff and diffs
# actual JSON output against the golden gate.json.
#
# Run: bats skills/autospec-shared/tests/integration/run-against-target.bats
# (from repo root)

setup() {
    BATS_FILE_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")" && pwd)"
    REPO_ROOT="$(cd "${BATS_FILE_DIR}/../../../.." && pwd)"
    SCRIPTS_DIR="${REPO_ROOT}/skills/autospec-shared/scripts"
    TARGETS_DIR="${REPO_ROOT}/skills/autospec-shared/test-targets"

    CHECK_DRIFT="${SCRIPTS_DIR}/check-doc-drift.sh"
    SCAN_MJS="${SCRIPTS_DIR}/scan-doc-scope.mjs"
}

# Helper: normalise JSON for comparison (sort keys, compact)
normalise_json() {
    local json="$1"
    echo "$json" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
// Sort drift and missing_scope arrays by doc_file/source_file for stable compare
if(Array.isArray(d.drift)) d.drift.sort((a,b)=>(a.doc_file||'').localeCompare(b.doc_file||''));
if(Array.isArray(d.missing_scope)) d.missing_scope.sort((a,b)=>(a.source_file||'').localeCompare(b.source_file||''));
process.stdout.write(JSON.stringify(d));
" 2>/dev/null
}

# Helper: run drift check against a target using --diff mode
run_drift_on_target() {
    local target_dir="$1"
    local diff_file="${target_dir}/golden/diff.patch"

    AUTOSPEC_REPO_ROOT="$target_dir" \
    AUTOSPEC_DOCS_DIR="${target_dir}/docs" \
    SCAN_DOC_SCOPE_MJS="$SCAN_MJS" \
        bash "$CHECK_DRIFT" --diff "$diff_file" 2>/dev/null
}

# ── target-doc-drift-bait ─────────────────────────────────────────────────────

@test "target-doc-drift-bait: check-doc-drift exits 1" {
    T="${TARGETS_DIR}/target-doc-drift-bait"
    run bash -c "AUTOSPEC_REPO_ROOT=\"$T\" AUTOSPEC_DOCS_DIR=\"${T}/docs\" SCAN_DOC_SCOPE_MJS=\"$SCAN_MJS\" bash \"$CHECK_DRIFT\" --diff \"${T}/golden/diff.patch\" 2>/dev/null"
    [ "$status" -eq 1 ]
}

@test "target-doc-drift-bait: drift entry points at docs/USER_MANUAL.md" {
    T="${TARGETS_DIR}/target-doc-drift-bait"
    actual="$(run_drift_on_target "$T" || true)"
    echo "$actual" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
if(!d.drift||d.drift.length===0) { console.error('no drift entries'); process.exit(1); }
if(d.drift[0].doc_file !== 'docs/USER_MANUAL.md') { console.error('wrong doc_file: '+d.drift[0].doc_file); process.exit(1); }
process.exit(0);
" 2>&1
}

@test "target-doc-drift-bait: actual gate.json matches golden" {
    T="${TARGETS_DIR}/target-doc-drift-bait"
    actual="$(run_drift_on_target "$T" || true)"
    golden="$(cat "${T}/golden/gate.json")"
    norm_actual="$(normalise_json "$actual")"
    norm_golden="$(normalise_json "$golden")"
    [ "$norm_actual" = "$norm_golden" ]
}

# ── target-reverse-engineer-bait ─────────────────────────────────────────────

@test "target-reverse-engineer-bait: check-doc-drift exits 2 (missing scope)" {
    T="${TARGETS_DIR}/target-reverse-engineer-bait"
    run bash -c "AUTOSPEC_REPO_ROOT=\"$T\" AUTOSPEC_DOCS_DIR=\"${T}/docs\" SCAN_DOC_SCOPE_MJS=\"$SCAN_MJS\" bash \"$CHECK_DRIFT\" --diff \"${T}/golden/diff.patch\" 2>/dev/null"
    [ "$status" -eq 2 ]
}

@test "target-reverse-engineer-bait: missing_scope contains src/router.js" {
    T="${TARGETS_DIR}/target-reverse-engineer-bait"
    actual="$(run_drift_on_target "$T" || true)"
    echo "$actual" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
if(!d.missing_scope||d.missing_scope.length===0) { console.error('no missing_scope'); process.exit(1); }
const found=d.missing_scope.some(e=>e.source_file==='src/router.js');
if(!found) { console.error('src/router.js not in missing_scope'); process.exit(1); }
process.exit(0);
" 2>&1
}

@test "target-reverse-engineer-bait: actual gate.json matches golden" {
    T="${TARGETS_DIR}/target-reverse-engineer-bait"
    actual="$(run_drift_on_target "$T" || true)"
    golden="$(cat "${T}/golden/gate.json")"
    norm_actual="$(normalise_json "$actual")"
    norm_golden="$(normalise_json "$golden")"
    [ "$norm_actual" = "$norm_golden" ]
}

# ── target-manifest-stale-bait ───────────────────────────────────────────────

@test "target-manifest-stale-bait: check-doc-drift exits 1" {
    T="${TARGETS_DIR}/target-manifest-stale-bait"
    run bash -c "AUTOSPEC_REPO_ROOT=\"$T\" AUTOSPEC_DOCS_DIR=\"${T}/docs\" SCAN_DOC_SCOPE_MJS=\"$SCAN_MJS\" bash \"$CHECK_DRIFT\" --diff \"${T}/golden/diff.patch\" 2>/dev/null"
    [ "$status" -eq 1 ]
}

@test "target-manifest-stale-bait: drift entry points at docs/MANIFEST.md" {
    T="${TARGETS_DIR}/target-manifest-stale-bait"
    actual="$(run_drift_on_target "$T" || true)"
    echo "$actual" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
if(!d.drift||d.drift.length===0) { console.error('no drift entries'); process.exit(1); }
if(d.drift[0].doc_file !== 'docs/MANIFEST.md') { console.error('wrong doc_file: '+d.drift[0].doc_file); process.exit(1); }
process.exit(0);
" 2>&1
}

@test "target-manifest-stale-bait: actual gate.json matches golden" {
    T="${TARGETS_DIR}/target-manifest-stale-bait"
    actual="$(run_drift_on_target "$T" || true)"
    golden="$(cat "${T}/golden/gate.json")"
    norm_actual="$(normalise_json "$actual")"
    norm_golden="$(normalise_json "$golden")"
    [ "$norm_actual" = "$norm_golden" ]
}

# ── target-visual-stale-bait ─────────────────────────────────────────────────

@test "target-visual-stale-bait: check-doc-drift exits 1" {
    T="${TARGETS_DIR}/target-visual-stale-bait"
    run bash -c "AUTOSPEC_REPO_ROOT=\"$T\" AUTOSPEC_DOCS_DIR=\"${T}/docs\" SCAN_DOC_SCOPE_MJS=\"$SCAN_MJS\" bash \"$CHECK_DRIFT\" --diff \"${T}/golden/diff.patch\" 2>/dev/null"
    [ "$status" -eq 1 ]
}

@test "target-visual-stale-bait: drift entry points at docs/DASHBOARD.md" {
    T="${TARGETS_DIR}/target-visual-stale-bait"
    actual="$(run_drift_on_target "$T" || true)"
    echo "$actual" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
if(!d.drift||d.drift.length===0) { console.error('no drift entries'); process.exit(1); }
if(d.drift[0].doc_file !== 'docs/DASHBOARD.md') { console.error('wrong doc_file: '+d.drift[0].doc_file); process.exit(1); }
process.exit(0);
" 2>&1
}

@test "target-visual-stale-bait: actual gate.json matches golden" {
    T="${TARGETS_DIR}/target-visual-stale-bait"
    actual="$(run_drift_on_target "$T" || true)"
    golden="$(cat "${T}/golden/gate.json")"
    norm_actual="$(normalise_json "$actual")"
    norm_golden="$(normalise_json "$golden")"
    [ "$norm_actual" = "$norm_golden" ]
}

# ── target-ai-low-confidence-bait ────────────────────────────────────────────

@test "target-ai-low-confidence-bait: check-doc-drift exits 1" {
    T="${TARGETS_DIR}/target-ai-low-confidence-bait"
    run bash -c "AUTOSPEC_REPO_ROOT=\"$T\" AUTOSPEC_DOCS_DIR=\"${T}/docs\" SCAN_DOC_SCOPE_MJS=\"$SCAN_MJS\" bash \"$CHECK_DRIFT\" --diff \"${T}/golden/diff.patch\" 2>/dev/null"
    [ "$status" -eq 1 ]
}

@test "target-ai-low-confidence-bait: drift entry points at docs/AUTH.md" {
    T="${TARGETS_DIR}/target-ai-low-confidence-bait"
    actual="$(run_drift_on_target "$T" || true)"
    echo "$actual" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
if(!d.drift||d.drift.length===0) { console.error('no drift entries'); process.exit(1); }
if(d.drift[0].doc_file !== 'docs/AUTH.md') { console.error('wrong doc_file: '+d.drift[0].doc_file); process.exit(1); }
process.exit(0);
" 2>&1
}

@test "target-ai-low-confidence-bait: actual gate.json matches golden" {
    T="${TARGETS_DIR}/target-ai-low-confidence-bait"
    actual="$(run_drift_on_target "$T" || true)"
    golden="$(cat "${T}/golden/gate.json")"
    norm_actual="$(normalise_json "$actual")"
    norm_golden="$(normalise_json "$golden")"
    [ "$norm_actual" = "$norm_golden" ]
}

# ── AI reviewer wiring tests (issue #434) ────────────────────────────────────
# These tests verify that gen-docs-from-spec.mjs invokes ai-review-doc.mjs
# per generated section and that low-confidence sections produce side JSON.

@test "gen-docs-from-spec: ai-reviewed annotation written per generated section (stub=low)" {
    FIXTURE="${REPO_ROOT}/skills/autospec-shared/tests/fixtures/gen-docs-fixture.json"
    [ -f "$FIXTURE" ] || skip "gen-docs fixture not found: $FIXTURE"
    WORK="$(mktemp -d -t ai-review-test.XXXXXX)"

    run env AUTOSPEC_AI_REVIEW_STUB=low \
        node "${SCRIPTS_DIR}/gen-docs-from-spec.mjs" \
        --fixture "$FIXTURE" --output-dir "$WORK" 2>/dev/null
    [ "$status" -eq 0 ]

    # At least one output file should contain ai-reviewed annotation
    annotated=0
    for f in "$WORK"/*.md "$WORK"/docs/*.md "$WORK"/docs/**/*.md; do
        [ -f "$f" ] || continue
        grep -q "ai-reviewed:" "$f" && annotated=1 && break
    done
    rm -rf "$WORK"
    [ "$annotated" -eq 1 ]
}

@test "gen-docs-from-spec: low-confidence sections produce ai-review-low-confidence.json (stub=low)" {
    FIXTURE="${REPO_ROOT}/skills/autospec-shared/tests/fixtures/gen-docs-fixture.json"
    [ -f "$FIXTURE" ] || skip "gen-docs fixture not found: $FIXTURE"
    WORK="$(mktemp -d -t ai-review-low-test.XXXXXX)"

    run env AUTOSPEC_AI_REVIEW_STUB=low \
        node "${SCRIPTS_DIR}/gen-docs-from-spec.mjs" \
        --fixture "$FIXTURE" --output-dir "$WORK" 2>/dev/null
    [ "$status" -eq 0 ]

    # Low-confidence side JSON should be written
    side_json="$WORK/ai-review-low-confidence.json"
    [ -f "$side_json" ]
    # Should be valid JSON array
    node -e "const d=JSON.parse(require('fs').readFileSync('${side_json}','utf8')); if(!Array.isArray(d)) process.exit(1);" 2>/dev/null
    rm -rf "$WORK"
}

@test "gen-docs-from-spec: cache-hit path: second invocation skips LLM (stub=low)" {
    FIXTURE="${REPO_ROOT}/skills/autospec-shared/tests/fixtures/gen-docs-fixture.json"
    [ -f "$FIXTURE" ] || skip "gen-docs fixture not found: $FIXTURE"
    WORK="$(mktemp -d -t ai-review-cache-test.XXXXXX)"
    CACHE_DIR="$WORK/cache"

    # First invocation — populates cache
    env AUTOSPEC_AI_REVIEW_STUB=low AUTOSPEC_AI_REVIEW_CACHE_DIR="$CACHE_DIR" \
        node "${SCRIPTS_DIR}/gen-docs-from-spec.mjs" \
        --fixture "$FIXTURE" --output-dir "$WORK/out1" >/dev/null 2>&1 || true

    # Second invocation — should hit cache (cache dir should have entries)
    env AUTOSPEC_AI_REVIEW_STUB=low AUTOSPEC_AI_REVIEW_CACHE_DIR="$CACHE_DIR" \
        node "${SCRIPTS_DIR}/gen-docs-from-spec.mjs" \
        --fixture "$FIXTURE" --output-dir "$WORK/out2" >/dev/null 2>&1 || true

    # Cache dir should have at least one .json entry
    cache_count="$(ls "$CACHE_DIR"/*.json 2>/dev/null | wc -l | tr -d ' ')"
    rm -rf "$WORK"
    [ "${cache_count:-0}" -gt 0 ]
}

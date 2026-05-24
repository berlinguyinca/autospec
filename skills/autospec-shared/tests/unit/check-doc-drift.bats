#!/usr/bin/env bats
# check-doc-drift.bats — Bats tests for check-doc-drift.sh
#
# Run: bats skills/autospec-shared/tests/unit/check-doc-drift.bats
# (from repo root, or adjust paths accordingly)

setup() {
    SCRIPT_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")/../.." && pwd)"
    CHECK_DRIFT="${SCRIPT_DIR}/scripts/check-doc-drift.sh"
    SCAN_MJS="${SCRIPT_DIR}/scripts/scan-doc-scope.mjs"
    FIXTURES_DIR="${SCRIPT_DIR}/tests/fixtures/doc-scope"

    TMP_DIR="$(mktemp -d /tmp/autospec-drifttest-XXXXXX)"
    export AUTOSPEC_REPO_ROOT="$TMP_DIR"
    export AUTOSPEC_DOCS_DIR="$TMP_DIR/docs"
    export SCAN_DOC_SCOPE_MJS="$SCAN_MJS"

    mkdir -p "$TMP_DIR/docs"
    mkdir -p "$TMP_DIR/skills/autospec-test/scripts"
    printf '#!/bin/bash\n# install\n' > "$TMP_DIR/install.sh"
    printf '#!/bin/bash\n# config\n' > "$TMP_DIR/skills/autospec-test/scripts/config.sh"
    printf '#!/bin/bash\n# test install\n' > "$TMP_DIR/skills/autospec-test/install.sh"
}

teardown() {
    rm -rf "$TMP_DIR"
}

# Helper: run script suppressing stderr (bats captures both stdout+stderr into $output)
run_drift() {
    run bash -c "\"$CHECK_DRIFT\" $* 2>/dev/null"
}

# Helper: parse JSON field from output
json_field() {
    local json="$1"
    local expr="$2"
    echo "$json" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
process.stdout.write(String($expr));
" 2>/dev/null || echo "PARSE_ERROR"
}

# ── Argument validation ───────────────────────────────────────────────────────

@test "no arguments prints error and exits 2" {
    run bash "$CHECK_DRIFT" 2>/dev/null
    [ "$status" -eq 2 ]
}

@test "--help exits 0" {
    run bash "$CHECK_DRIFT" --help 2>/dev/null
    [ "$status" -eq 0 ]
    [[ "$output" =~ "Usage" ]]
}

@test "--diff with missing file exits 2" {
    run bash "$CHECK_DRIFT" --diff /tmp/nonexistent-autospec-drift-xyz.patch 2>/dev/null
    [ "$status" -eq 2 ]
}

@test "unknown option exits 2" {
    run bash "$CHECK_DRIFT" --bogus-option 2>/dev/null
    [ "$status" -eq 2 ]
}

# ── Empty diff ────────────────────────────────────────────────────────────────

@test "empty diff passes with exit 0 and valid JSON" {
    DFILE="$(mktemp /tmp/autospec-driftempty-XXXXXX)"
    printf "" > "$DFILE"
    run bash -c "\"$CHECK_DRIFT\" --diff \"$DFILE\" 2>/dev/null"
    rm -f "$DFILE"
    [ "$status" -eq 0 ]
    passed="$(json_field "$output" "d.passed")"
    [ "$passed" = "true" ]
}

# ── Working tree mode ─────────────────────────────────────────────────────────

@test "--working-tree on clean git repo exits 0" {
    git -C "$TMP_DIR" init -q
    git -C "$TMP_DIR" config user.email "test@test.com"
    git -C "$TMP_DIR" config user.name "Test"
    git -C "$TMP_DIR" add -A
    git -C "$TMP_DIR" commit -q -m "init"
    run bash -c "\"$CHECK_DRIFT\" --working-tree 2>/dev/null"
    [ "$status" -eq 0 ]
}

# ── JSON output schema ────────────────────────────────────────────────────────

@test "output JSON has all required fields" {
    DFILE="$(mktemp /tmp/autospec-driftempty-XXXXXX)"
    printf "" > "$DFILE"
    run bash -c "\"$CHECK_DRIFT\" --diff \"$DFILE\" 2>/dev/null"
    rm -f "$DFILE"
    [ "$status" -eq 0 ]
    result="$(json_field "$output" "
typeof d.passed==='boolean' &&
Array.isArray(d.drift) &&
Array.isArray(d.missing_scope) &&
Array.isArray(d.visual_stale) &&
Array.isArray(d.ai_review_stale) &&
typeof d.skipped==='boolean' ? 'ok' : 'fail'
")"
    [ "$result" = "ok" ]
}

# ── Missing scope detection ───────────────────────────────────────────────────

@test "changed source with no doc scope → missing_scope + exit 2" {
    cp "$FIXTURES_DIR/no-scope.md" "$TMP_DIR/docs/USER_MANUAL.md"

    DFILE="$(mktemp /tmp/autospec-driftpatch-XXXXXX)"
    cat > "$DFILE" <<'DIFF'
diff --git a/install.sh b/install.sh
index abc..def 100755
--- a/install.sh
+++ b/install.sh
@@ -1,2 +1,3 @@
 #!/bin/bash
 # install script
+echo "new line"
DIFF

    run bash -c "\"$CHECK_DRIFT\" --diff \"$DFILE\" 2>/dev/null"
    rm -f "$DFILE"
    [ "$status" -eq 2 ]
    mc="$(json_field "$output" "d.missing_scope.length")"
    [ "$mc" -gt 0 ]
}

# ── Drift detection ───────────────────────────────────────────────────────────

@test "source matches scope but doc not updated → drift + exit 1" {
    cp "$FIXTURES_DIR/valid-scope.md" "$TMP_DIR/docs/USER_MANUAL.md"

    DFILE="$(mktemp /tmp/autospec-driftpatch-XXXXXX)"
    cat > "$DFILE" <<'DIFF'
diff --git a/install.sh b/install.sh
index abc..def 100755
--- a/install.sh
+++ b/install.sh
@@ -1,2 +1,3 @@
 #!/bin/bash
 # install script
+echo "new line"
DIFF

    run bash -c "\"$CHECK_DRIFT\" --diff \"$DFILE\" 2>/dev/null"
    rm -f "$DFILE"
    [ "$status" -eq 1 ]
    dc="$(json_field "$output" "d.drift.length")"
    [ "$dc" -gt 0 ]
}

@test "drift entry has required doc_file, heading, matching_source_files fields" {
    cp "$FIXTURES_DIR/valid-scope.md" "$TMP_DIR/docs/USER_MANUAL.md"

    DFILE="$(mktemp /tmp/autospec-driftpatch-XXXXXX)"
    cat > "$DFILE" <<'DIFF'
diff --git a/install.sh b/install.sh
index abc..def 100755
--- a/install.sh
+++ b/install.sh
@@ -1 +1,2 @@
 #!/bin/bash
+echo hi
DIFF

    run bash -c "\"$CHECK_DRIFT\" --diff \"$DFILE\" 2>/dev/null"
    rm -f "$DFILE"
    result="$(json_field "$output" "
d.drift.length===0 ? 'no-drift' :
(typeof d.drift[0].doc_file==='string' &&
 typeof d.drift[0].heading==='string' &&
 Array.isArray(d.drift[0].matching_source_files)) ? 'ok' : 'fail'
")"
    [ "$result" = "ok" ]
}

# ── Clean path ────────────────────────────────────────────────────────────────

@test "source matches scope AND doc updated → passed:true exit 0" {
    cp "$FIXTURES_DIR/valid-scope.md" "$TMP_DIR/docs/USER_MANUAL.md"

    DFILE="$(mktemp /tmp/autospec-driftpatch-XXXXXX)"
    cat > "$DFILE" <<'DIFF'
diff --git a/install.sh b/install.sh
index abc..def 100755
--- a/install.sh
+++ b/install.sh
@@ -1 +1,2 @@
 #!/bin/bash
+echo hi
diff --git a/docs/USER_MANUAL.md b/docs/USER_MANUAL.md
index 111..222 100644
--- a/docs/USER_MANUAL.md
+++ b/docs/USER_MANUAL.md
@@ -9,3 +9,4 @@ To install, run the installer script.

 To install, run the installer script.
+Updated install instructions here.
DIFF

    run bash -c "\"$CHECK_DRIFT\" --diff \"$DFILE\" 2>/dev/null"
    rm -f "$DFILE"
    [ "$status" -eq 0 ]
    passed="$(json_field "$output" "d.passed")"
    [ "$passed" = "true" ]
}

# ── docs: skip pattern matching ───────────────────────────────────────────────

@test "exact 'docs: skip' matches skip pattern" {
    run bash -c 'echo "docs: skip" | grep -qiE "^docs:[[:space:]]*skip[[:space:]]*$"'
    [ "$status" -eq 0 ]
}

@test "docs:SKIP (uppercase) matches skip pattern" {
    run bash -c 'echo "docs: SKIP" | grep -qiE "^docs:[[:space:]]*skip[[:space:]]*$"'
    [ "$status" -eq 0 ]
}

@test "docs: skip with trailing spaces matches" {
    run bash -c 'echo "docs: skip   " | grep -qiE "^docs:[[:space:]]*skip[[:space:]]*$"'
    [ "$status" -eq 0 ]
}

@test "partial match 'docs: skip me' does NOT trigger skip" {
    run bash -c 'echo "docs: skip me" | grep -qiE "^docs:[[:space:]]*skip[[:space:]]*$"'
    [ "$status" -ne 0 ]
}

# ── Cross-doc source attribution (multi-file diff) ────────────────────────────

# Regression: per-doc matching-sources state must not leak across docs.
# A source matched for doc A's section (entry index 0) must NOT be re-attributed
# to doc B's section at the same entry index when doc B's globs match nothing in
# the diff. Pre-fix, matching_sources_0.txt persisted across docs in the shared
# work dir, so doc B inherited doc A's matches → false drift on the wrong doc.
@test "multi-file diff does not leak sources across docs (no cross-attribution)" {
    mkdir -p "$TMP_DIR/pkg"
    printf '#!/bin/bash\n' > "$TMP_DIR/pkg/p.sh"
    printf '#!/bin/bash\n' > "$TMP_DIR/pkg/q.sh"

    # Two docs, mirrored so that a leak surfaces regardless of find() ordering:
    #   Doc P: entry 0 matches pkg/p.sh, entry 1 matches nothing in the diff.
    #   Doc Q: entry 0 matches nothing in the diff, entry 1 matches pkg/q.sh.
    # Pre-fix, the shared matching_sources_<idx>.txt files were never reset
    # between docs, so the second-processed doc's empty-match section at entry N
    # inherited the matches the first-processed doc wrote at entry N — yielding a
    # drift entry attributing pkg/p.sh to Doc Q (or pkg/q.sh to Doc P).
    cat > "$TMP_DIR/docs/DOC_P.md" <<'DOCP'
# Doc P

## P Match

<!-- autospec-doc-scope:
  src: ["pkg/p.sh"]
  reason: "Doc P tracks p.sh"
-->

p content

## P Empty

<!-- autospec-doc-scope:
  src: ["unmatched/zzz-p.sh"]
  reason: "Doc P entry that matches nothing in the diff"
-->

p empty
DOCP

    cat > "$TMP_DIR/docs/DOC_Q.md" <<'DOCQ'
# Doc Q

## Q Empty

<!-- autospec-doc-scope:
  src: ["unmatched/zzz-q.sh"]
  reason: "Doc Q entry that matches nothing in the diff"
-->

q empty

## Q Match

<!-- autospec-doc-scope:
  src: ["pkg/q.sh"]
  reason: "Doc Q tracks q.sh"
-->

q content
DOCQ

    # Multi-file source diff touching pkg/p.sh and pkg/q.sh (both covered, so no
    # missing_scope). Neither doc is updated → each legitimately drifts on its own
    # source. The leak would add a cross-attributed entry.
    DFILE="$(mktemp /tmp/autospec-driftpatch-XXXXXX)"
    cat > "$DFILE" <<'DIFF'
diff --git a/pkg/p.sh b/pkg/p.sh
index abc..def 100755
--- a/pkg/p.sh
+++ b/pkg/p.sh
@@ -1 +1,2 @@
 #!/bin/bash
+echo p
diff --git a/pkg/q.sh b/pkg/q.sh
index abc..def 100755
--- a/pkg/q.sh
+++ b/pkg/q.sh
@@ -1 +1,2 @@
 #!/bin/bash
+echo q
DIFF

    run bash -c "\"$CHECK_DRIFT\" --diff \"$DFILE\" 2>/dev/null"
    rm -f "$DFILE"

    # Doc P must never list pkg/q.sh; Doc Q must never list pkg/p.sh.
    leak="$(json_field "$output" "
[].concat(d.drift, d.drift_warn).some(e =>
  (e.doc_file === 'docs/DOC_P.md' && (e.matching_source_files||[]).includes('pkg/q.sh')) ||
  (e.doc_file === 'docs/DOC_Q.md' && (e.matching_source_files||[]).includes('pkg/p.sh'))
) ? 'LEAK' : 'clean'
")"
    [ "$leak" = "clean" ]

    # Sanity: each doc's own source is still attributed correctly (single-file
    # behavior preserved on the matching sections).
    own="$(json_field "$output" "
[].concat(d.drift, d.drift_warn).some(e =>
  e.doc_file === 'docs/DOC_P.md' && (e.matching_source_files||[]).includes('pkg/p.sh')
) &&
[].concat(d.drift, d.drift_warn).some(e =>
  e.doc_file === 'docs/DOC_Q.md' && (e.matching_source_files||[]).includes('pkg/q.sh')
) ? 'ok' : 'fail'
")"
    [ "$own" = "ok" ]
}

# ── No docs directory ─────────────────────────────────────────────────────────

@test "no docs directory + changed source → missing_scope + exit 2" {
    rm -rf "$TMP_DIR/docs"

    DFILE="$(mktemp /tmp/autospec-driftpatch-XXXXXX)"
    cat > "$DFILE" <<'DIFF'
diff --git a/install.sh b/install.sh
index abc..def 100755
--- a/install.sh
+++ b/install.sh
@@ -1 +1,2 @@
 #!/bin/bash
+echo hi
DIFF

    run bash -c "\"$CHECK_DRIFT\" --diff \"$DFILE\" 2>/dev/null"
    rm -f "$DFILE"
    [ "$status" -eq 2 ]
}

@test "empty docs dir + no source changes → exit 0" {
    DFILE="$(mktemp /tmp/autospec-driftempty-XXXXXX)"
    printf "" > "$DFILE"
    run bash -c "\"$CHECK_DRIFT\" --diff \"$DFILE\" 2>/dev/null"
    rm -f "$DFILE"
    [ "$status" -eq 0 ]
}

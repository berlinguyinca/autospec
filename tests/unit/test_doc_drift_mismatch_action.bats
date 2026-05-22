#!/usr/bin/env bats
# tests/unit/test_doc_drift_mismatch_action.bats
# Unit tests for mismatch_action: warn | hard_fail support in
#   scan-doc-scope.mjs and check-doc-drift.sh

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCAN_MJS="$REPO_ROOT/skills/autospec-shared/scripts/scan-doc-scope.mjs"
    DRIFT_SH="$REPO_ROOT/skills/autospec-shared/scripts/check-doc-drift.sh"

    # Temp workspace for each test
    WORK="$(mktemp -d)"
}

teardown() {
    rm -rf "$WORK"
}

# ── scan-doc-scope.mjs: mismatch_action parsing ───────────────────────────────

@test "scan-doc-scope: mismatch_action: warn is parsed and returned" {
    cat > "$WORK/doc.md" <<'MD'
# Section

<!-- autospec-doc-scope:
  src: ["foo.sh"]
  mismatch_action: warn
-->

content
MD
    run node "$SCAN_MJS" "$WORK/doc.md"
    [ "$status" -eq 0 ]
    echo "$output" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
if (d[0].mismatch_action !== 'warn') { console.error('expected warn, got', d[0].mismatch_action); process.exit(1); }
"
}

@test "scan-doc-scope: missing mismatch_action defaults to hard_fail" {
    cat > "$WORK/doc.md" <<'MD'
# Section

<!-- autospec-doc-scope:
  src: ["foo.sh"]
-->

content
MD
    run node "$SCAN_MJS" "$WORK/doc.md"
    [ "$status" -eq 0 ]
    echo "$output" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
if (d[0].mismatch_action !== 'hard_fail') { console.error('expected hard_fail, got', d[0].mismatch_action); process.exit(1); }
"
}

@test "scan-doc-scope: mismatch_action: hard_fail is parsed and returned" {
    cat > "$WORK/doc.md" <<'MD'
# Section

<!-- autospec-doc-scope:
  src: ["bar.sh"]
  mismatch_action: hard_fail
-->

content
MD
    run node "$SCAN_MJS" "$WORK/doc.md"
    [ "$status" -eq 0 ]
    echo "$output" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
if (d[0].mismatch_action !== 'hard_fail') { console.error('expected hard_fail, got', d[0].mismatch_action); process.exit(1); }
"
}

# ── check-doc-drift.sh: warn-only scope does not block ───────────────────────

@test "check-doc-drift: drift in warn scope → exit 0 + drift_warn populated" {
    # Set up a fake repo with a doc that has mismatch_action: warn scope
    mkdir -p "$WORK/repo/docs" "$WORK/repo/.git"
    git -C "$WORK/repo" init -q
    git -C "$WORK/repo" -c user.email="test@test.com" -c user.name="Test" commit --allow-empty -m "init"

    cat > "$WORK/repo/docs/MANUAL.md" <<'MD'
# Manual

<!-- autospec-doc-scope:
  src: ["src/foo.sh"]
  reason: "over-broad scope"
  mismatch_action: warn
-->

Some content here.
MD

    # Diff: src/foo.sh changed, doc section NOT updated
    cat > "$WORK/diff.txt" <<'DIFF'
diff --git a/src/foo.sh b/src/foo.sh
index 0000000..1111111 100644
--- a/src/foo.sh
+++ b/src/foo.sh
@@ -1,3 +1,4 @@
 #!/usr/bin/env bash
+# new line
 echo hello
DIFF

    AUTOSPEC_REPO_ROOT="$WORK/repo" \
    AUTOSPEC_DOCS_DIR="$WORK/repo/docs" \
    SCAN_DOC_SCOPE_MJS="$SCAN_MJS" \
    run bash "$DRIFT_SH" --diff "$WORK/diff.txt"

    [ "$status" -eq 0 ]
    echo "$output" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
if (!Array.isArray(d.drift_warn) || d.drift_warn.length === 0) {
  console.error('drift_warn should be non-empty'); process.exit(1);
}
if (d.drift.length !== 0) {
  console.error('drift (hard_fail) should be empty'); process.exit(1);
}
if (d.passed !== true) {
  console.error('passed should be true for warn-only drift'); process.exit(1);
}
"
}

@test "check-doc-drift: drift in hard_fail scope → exit 1 + drift populated" {
    mkdir -p "$WORK/repo/docs" "$WORK/repo/.git"
    git -C "$WORK/repo" init -q
    git -C "$WORK/repo" -c user.email="test@test.com" -c user.name="Test" commit --allow-empty -m "init"

    cat > "$WORK/repo/docs/MANUAL.md" <<'MD'
# Manual

<!-- autospec-doc-scope:
  src: ["src/bar.sh"]
  reason: "strict scope"
-->

Some content here.
MD

    cat > "$WORK/diff.txt" <<'DIFF'
diff --git a/src/bar.sh b/src/bar.sh
index 0000000..1111111 100644
--- a/src/bar.sh
+++ b/src/bar.sh
@@ -1,3 +1,4 @@
 #!/usr/bin/env bash
+# new line
 echo hello
DIFF

    AUTOSPEC_REPO_ROOT="$WORK/repo" \
    AUTOSPEC_DOCS_DIR="$WORK/repo/docs" \
    SCAN_DOC_SCOPE_MJS="$SCAN_MJS" \
    run bash "$DRIFT_SH" --diff "$WORK/diff.txt"

    [ "$status" -eq 1 ]
    echo "$output" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
if (!Array.isArray(d.drift) || d.drift.length === 0) {
  console.error('drift should be non-empty'); process.exit(1);
}
if (d.passed !== false) {
  console.error('passed should be false for hard_fail drift'); process.exit(1);
}
"
}

@test "check-doc-drift: no drift → exit 0 + both drift arrays empty" {
    mkdir -p "$WORK/repo/docs" "$WORK/repo/.git"
    git -C "$WORK/repo" init -q
    git -C "$WORK/repo" -c user.email="test@test.com" -c user.name="Test" commit --allow-empty -m "init"

    cat > "$WORK/repo/docs/MANUAL.md" <<'MD'
# Manual

<!-- autospec-doc-scope:
  src: ["src/baz.sh"]
  reason: "strict scope"
-->

Some content here.
MD

    # Empty diff (no source changes)
    cat > "$WORK/diff.txt" <<'DIFF'
DIFF

    AUTOSPEC_REPO_ROOT="$WORK/repo" \
    AUTOSPEC_DOCS_DIR="$WORK/repo/docs" \
    SCAN_DOC_SCOPE_MJS="$SCAN_MJS" \
    run bash "$DRIFT_SH" --diff "$WORK/diff.txt"

    [ "$status" -eq 0 ]
    echo "$output" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
if (d.drift.length !== 0) { console.error('drift should be empty'); process.exit(1); }
if (d.drift_warn.length !== 0) { console.error('drift_warn should be empty'); process.exit(1); }
if (d.passed !== true) { console.error('passed should be true'); process.exit(1); }
"
}

# ── syntax check ─────────────────────────────────────────────────────────────

@test "check-doc-drift.sh: bash -n exits 0" {
    run bash -n "$DRIFT_SH"
    [ "$status" -eq 0 ]
}

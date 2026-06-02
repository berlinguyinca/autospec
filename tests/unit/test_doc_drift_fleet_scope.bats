#!/usr/bin/env bats
# tests/unit/test_doc_drift_fleet_scope.bats
#
# Regression tests for issue #851: docs-drift gate false-tripped (exit 2) on
# PRs touching skills/autospec-fleet/scripts/**, skills/autospec-fleet/gui/**,
# and tests/fleet/** (or any tests/**).
#
# Two fixes ship together:
#   1. check-doc-drift.sh excludes tests/** from the source set so test-only
#      diffs never trigger missing-scope (Option b).
#   2. docs/USER_MANUAL.md has an autospec-doc-scope annotation whose src:
#      globs cover skills/autospec-fleet/scripts/** and
#      skills/autospec-fleet/gui/** so fleet-GUI diffs return exit 0/1
#      (covered) rather than exit 2 (Option a).
#
# Each test builds a synthetic diff, runs check-doc-drift.sh against the real
# docs/ tree, and asserts the expected exit code.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCAN_MJS="$REPO_ROOT/skills/autospec-shared/scripts/scan-doc-scope.mjs"
    DRIFT_SH="$REPO_ROOT/skills/autospec-shared/scripts/check-doc-drift.sh"
    WORK="$(mktemp -d)"
}

teardown() {
    rm -rf "$WORK"
}

# Helper: run check-doc-drift.sh against the REAL docs/ tree using a diff file.
run_drift() {
    local diff_file="$1"
    SCAN_DOC_SCOPE_MJS="$SCAN_MJS" \
    AUTOSPEC_REPO_ROOT="$REPO_ROOT" \
    AUTOSPEC_DOCS_DIR="$REPO_ROOT/docs" \
    run bash "$DRIFT_SH" --diff "$diff_file"
}

# ── Regression A: tests-only diff must NOT return exit 2 ─────────────────────
#
# Before the fix: a diff touching only tests/fleet/test_fleet_gui.bats returned
# exit 2 (missing-scope), falsely blocking the gate signal.  After the fix:
# tests/** is excluded from the source set, so this diff returns exit 0 (clean,
# no source files to check).
#
# Mutation-verified: removing the tests/ exclusion from check-doc-drift.sh
# makes this test FAIL (exit 2 instead of exit 0).

@test "tests-only diff (tests/fleet/**) does NOT return exit 2 (#851)" {
    cat > "$WORK/diff.txt" <<'DIFF'
diff --git a/tests/fleet/test_fleet_gui.bats b/tests/fleet/test_fleet_gui.bats
index 0000000..1111111 100644
--- a/tests/fleet/test_fleet_gui.bats
+++ b/tests/fleet/test_fleet_gui.bats
@@ -1,3 +1,4 @@
 #!/usr/bin/env bats
+# regression test added
 echo hello
DIFF

    run_drift "$WORK/diff.txt"

    # Must NOT be exit 2 (missing-scope). Exit 0 = clean (no source files to
    # check); exit 1 = drift found but correctly categorized. Either is fine.
    [ "$status" -ne 2 ]
}

@test "tests/** diff returns exit 0 (tests excluded from source set) (#851)" {
    cat > "$WORK/diff.txt" <<'DIFF'
diff --git a/tests/unit/test_some_unit.bats b/tests/unit/test_some_unit.bats
index 0000000..1111111 100644
--- a/tests/unit/test_some_unit.bats
+++ b/tests/unit/test_some_unit.bats
@@ -1,3 +1,4 @@
 #!/usr/bin/env bats
+# new unit test
 echo hello
DIFF

    run_drift "$WORK/diff.txt"

    # Tests-only diff: no documentable source file → exit 0 (clean/skipped).
    [ "$status" -eq 0 ]
    # Confirm missing_scope array is empty (tests path was excluded, not missed).
    echo "$output" | python3 -c "
import json, sys
d = json.loads(sys.stdin.read())
ms = d.get('missing_scope', [])
assert ms == [], f'expected empty missing_scope, got: {ms}'
"
}

# ── Regression B: fleet scripts diff must return exit 0/1, not exit 2 ────────
#
# Before the fix: a diff touching skills/autospec-fleet/scripts/fleet-gui-server.py
# returned exit 2 (no scope declared). After the fix: a doc-scope annotation in
# docs/USER_MANUAL.md covers skills/autospec-fleet/scripts/** so the file is
# "covered" — drift gate can legitimately return 0 (section updated) or 1 (drift)
# but never 2 (missing-scope).
#
# Mutation-verified: removing the fleet-gui src: globs from the doc-scope
# annotation in USER_MANUAL.md makes this test FAIL (exit 2 instead of 0/1).

@test "fleet scripts diff (skills/autospec-fleet/scripts/**) does NOT return exit 2 (#851)" {
    cat > "$WORK/diff.txt" <<'DIFF'
diff --git a/skills/autospec-fleet/scripts/fleet-gui-server.py b/skills/autospec-fleet/scripts/fleet-gui-server.py
index 0000000..1111111 100644
--- a/skills/autospec-fleet/scripts/fleet-gui-server.py
+++ b/skills/autospec-fleet/scripts/fleet-gui-server.py
@@ -1,3 +1,4 @@
 #!/usr/bin/env python3
+# new comment
 import sys
DIFF

    run_drift "$WORK/diff.txt"

    # Must NOT be exit 2.
    [ "$status" -ne 2 ]
}

@test "fleet GUI diff (skills/autospec-fleet/gui/**) does NOT return exit 2 (#851)" {
    cat > "$WORK/diff.txt" <<'DIFF'
diff --git a/skills/autospec-fleet/gui/index.html b/skills/autospec-fleet/gui/index.html
index 0000000..1111111 100644
--- a/skills/autospec-fleet/gui/index.html
+++ b/skills/autospec-fleet/gui/index.html
@@ -1,3 +1,4 @@
 <!DOCTYPE html>
+<!-- updated -->
 <html>
DIFF

    run_drift "$WORK/diff.txt"

    # Must NOT be exit 2.
    [ "$status" -ne 2 ]
}

# ── Sanity: non-fleet non-test source file still trips correctly ──────────────
#
# Verify the exclusion didn't swallow ALL source files. A diff touching a
# source file that genuinely has no doc-scope should still return exit 2.
# This guards against an overly broad exclusion that mutes the gate entirely.

@test "genuinely uncovered source file still returns exit 2 (gate still works) (#851)" {
    cat > "$WORK/diff.txt" <<'DIFF'
diff --git a/some/totally/new/uncovered-script.sh b/some/totally/new/uncovered-script.sh
index 0000000..1111111 100644
--- /dev/null
+++ b/some/totally/new/uncovered-script.sh
@@ -0,0 +1,3 @@
+#!/usr/bin/env bash
+# brand new script not covered by any scope
+echo hello
DIFF

    run_drift "$WORK/diff.txt"

    # This path matches no doc-scope → exit 2 (missing-scope still fires).
    [ "$status" -eq 2 ]
    echo "$output" | python3 -c "
import json, sys
d = json.loads(sys.stdin.read())
ms = d.get('missing_scope', [])
assert len(ms) > 0, f'expected non-empty missing_scope for genuinely uncovered file'
"
}

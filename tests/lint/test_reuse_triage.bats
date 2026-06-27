#!/usr/bin/env bats
# tests/lint/test_reuse_triage.bats — reuse-triage RULE_IDs:
#   REINVENT_REPO_UTIL, NEW_DEP_UNJUSTIFIED, NEW_ABSTRACTION_SINGLE_CALLER
#
# TDD: written before detectors exist.  Run with:
#   bats tests/lint/test_reuse_triage.bats

setup() {
    REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
    LINT="${REPO_ROOT}/scripts/lint-implementation.sh"
    TMPDIR_BATS="$(mktemp -d)"
    FIXTURES="${BATS_TEST_DIRNAME}/fixtures"
}

teardown() {
    rm -rf "$TMPDIR_BATS"
}

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# make_diff <rel-path> <src-file> <out-diff>
# Build a synthetic "new file" git diff from a source payload.
make_diff() {
    local rel="$1" src="$2" out="$3"
    local nlines
    nlines="$(wc -l < "$src" | tr -d ' ')"
    {
        printf 'diff --git a/%s b/%s\n' "$rel" "$rel"
        printf 'new file mode 100644\n'
        printf '%s\n' '--- /dev/null'
        printf '+++ b/%s\n' "$rel"
        printf '@@ -0,0 +1,%s @@\n' "$nlines"
        sed 's/^/+/' "$src"
    } > "$out"
}

# make_hunk_diff <rel-path> <content> <out-diff>
# Build a diff with a single +hunk (no new-file mode line) from raw content string.
make_hunk_diff() {
    local rel="$1" content="$2" out="$3"
    local nlines
    nlines="$(printf '%s\n' "$content" | wc -l | tr -d ' ')"
    {
        printf 'diff --git a/%s b/%s\n' "$rel" "$rel"
        printf '%s\n' '--- a/'"$rel"
        printf '+++ b/%s\n' "$rel"
        printf '@@ -1,0 +1,%s @@\n' "$nlines"
        printf '%s\n' "$content" | sed 's/^/+/'
    } > "$out"
}

# ---------------------------------------------------------------------------
# REINVENT_REPO_UTIL
# ---------------------------------------------------------------------------

@test "REINVENT_REPO_UTIL: new function whose name already exists in scripts/ is flagged" {
    # The fixture adds a function called 'log_info' — which already appears in
    # scripts/lib/ (or we plant a seed file so rg finds it).
    seed_dir="$TMPDIR_BATS/scripts/lib"
    mkdir -p "$seed_dir"
    # Plant an existing definition so rg can find it.
    printf 'log_info() { printf "[INFO] %s\\n" "$@"; }\n' > "$seed_dir/logging.sh"

    src="$TMPDIR_BATS/new_script.sh"
    cat > "$src" <<'PAYLOAD'
#!/usr/bin/env bash
# linter:allow-SECURITY fixture only
log_info() {
    printf '[INFO] %s\n' "$@"
}
PAYLOAD

    diff="$TMPDIR_BATS/reinvent.diff"
    make_diff "scripts/new_script.sh" "$src" "$diff"

    # Run the linter with RG_ROOT pointing at our temp tree so rg finds the seed.
    run env LINT_SEARCH_ROOT="$TMPDIR_BATS" bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    printf '%s\n' "$output" | grep -q '^REINVENT_REPO_UTIL:'
}

@test "REINVENT_REPO_UTIL: new function not present anywhere else emits no finding" {
    src="$TMPDIR_BATS/unique_func.sh"
    cat > "$src" <<'PAYLOAD'
#!/usr/bin/env bash
xyzzy_completely_unique_helper_1439() {
    printf 'hello\n'
}
PAYLOAD

    diff="$TMPDIR_BATS/unique.diff"
    make_diff "scripts/unique_func.sh" "$src" "$diff"

    run bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    count="$(printf '%s\n' "$output" | grep -c '^REINVENT_REPO_UTIL:' || true)"
    [ "$count" -eq 0 ]
}

@test "REINVENT_REPO_UTIL: linter:allow suppresses finding" {
    seed_dir="$TMPDIR_BATS/scripts/lib"
    mkdir -p "$seed_dir"
    printf 'log_info() { printf "[INFO] %s\\n" "$@"; }\n' > "$seed_dir/logging.sh"

    src="$TMPDIR_BATS/allowed_script.sh"
    cat > "$src" <<'PAYLOAD'
#!/usr/bin/env bash
# linter:allow-REINVENT_REPO_UTIL intentional reimplementation with different format
log_info() {
    printf '[INFO][%s] %s\n' "$(date +%s)" "$@"
}
PAYLOAD

    diff="$TMPDIR_BATS/allowed.diff"
    make_diff "scripts/allowed_script.sh" "$src" "$diff"

    run env LINT_SEARCH_ROOT="$TMPDIR_BATS" bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    count="$(printf '%s\n' "$output" | grep -c '^REINVENT_REPO_UTIL:' || true)"
    [ "$count" -eq 0 ]
}

@test "REINVENT_REPO_UTIL: rg failure causes fail-open (no finding, exit 0)" {
    src="$TMPDIR_BATS/any_func.sh"
    cat > "$src" <<'PAYLOAD'
#!/usr/bin/env bash
some_func() { :; }
PAYLOAD

    diff="$TMPDIR_BATS/fail_open.diff"
    make_diff "scripts/any_func.sh" "$src" "$diff"

    # Point search root at a nonexistent path so rg errors out.
    run env LINT_SEARCH_ROOT="/nonexistent_path_1439" bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    count="$(printf '%s\n' "$output" | grep -c '^REINVENT_REPO_UTIL:' || true)"
    [ "$count" -eq 0 ]
    [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------------------
# NEW_DEP_UNJUSTIFIED
# ---------------------------------------------------------------------------

@test "NEW_DEP_UNJUSTIFIED: dep added to package.json without why: marker is flagged" {
    diff="$TMPDIR_BATS/dep_add.diff"
    cat > "$diff" <<'DIFF'
diff --git a/package.json b/package.json
--- a/package.json
+++ b/package.json
@@ -1,4 +1,5 @@
 {
   "dependencies": {
+    "some-lib": "^1.2.3"
   }
 }
DIFF

    run bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    printf '%s\n' "$output" | grep -q '^NEW_DEP_UNJUSTIFIED:'
}

@test "NEW_DEP_UNJUSTIFIED: dep added with why: comment is silent" {
    diff="$TMPDIR_BATS/dep_why.diff"
    cat > "$diff" <<'DIFF'
diff --git a/package.json b/package.json
--- a/package.json
+++ b/package.json
@@ -1,4 +1,6 @@
 {
   "dependencies": {
+    "some-lib": "^1.2.3"
+  }
+  // why: needed for HTTP client pooling in scripts/fetch.js
 }
DIFF

    run bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    count="$(printf '%s\n' "$output" | grep -c '^NEW_DEP_UNJUSTIFIED:' || true)"
    [ "$count" -eq 0 ]
}

@test "NEW_DEP_UNJUSTIFIED: dep added to requirements.txt without why: is flagged" {
    diff="$TMPDIR_BATS/req_add.diff"
    cat > "$diff" <<'DIFF'
diff --git a/requirements.txt b/requirements.txt
--- a/requirements.txt
+++ b/requirements.txt
@@ -1,3 +1,4 @@
 requests==2.28.0
+boto3==1.26.0
 pytest==7.2.0
DIFF

    run bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    printf '%s\n' "$output" | grep -q '^NEW_DEP_UNJUSTIFIED:'
}

@test "NEW_DEP_UNJUSTIFIED: dep in requirements.txt with why: comment in same hunk is silent" {
    diff="$TMPDIR_BATS/req_why.diff"
    cat > "$diff" <<'DIFF'
diff --git a/requirements.txt b/requirements.txt
--- a/requirements.txt
+++ b/requirements.txt
@@ -1,3 +1,5 @@
 requests==2.28.0
+# why: boto3 needed for S3 uploads in scripts/upload.py
+boto3==1.26.0
 pytest==7.2.0
DIFF

    run bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    count="$(printf '%s\n' "$output" | grep -c '^NEW_DEP_UNJUSTIFIED:' || true)"
    [ "$count" -eq 0 ]
}

# ---------------------------------------------------------------------------
# NEW_ABSTRACTION_SINGLE_CALLER
# ---------------------------------------------------------------------------

@test "NEW_ABSTRACTION_SINGLE_CALLER: new *-manager file with one call site is flagged" {
    diff="$TMPDIR_BATS/manager_add.diff"
    cat > "$diff" <<'DIFF'
diff --git a/scripts/worker-manager.sh b/scripts/worker-manager.sh
new file mode 100644
--- /dev/null
+++ b/scripts/worker-manager.sh
@@ -0,0 +1,5 @@
+#!/usr/bin/env bash
+manage_workers() {
+    printf 'managing\n'
+}
diff --git a/scripts/run.sh b/scripts/run.sh
--- a/scripts/run.sh
+++ b/scripts/run.sh
@@ -10,0 +11 @@
+manage_workers
DIFF

    run bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    printf '%s\n' "$output" | grep -q '^NEW_ABSTRACTION_SINGLE_CALLER:'
}

@test "NEW_ABSTRACTION_SINGLE_CALLER: new *-manager file with zero call sites in diff is flagged" {
    diff="$TMPDIR_BATS/manager_nocall.diff"
    cat > "$diff" <<'DIFF'
diff --git a/scripts/session-manager.sh b/scripts/session-manager.sh
new file mode 100644
--- /dev/null
+++ b/scripts/session-manager.sh
@@ -0,0 +1,4 @@
+#!/usr/bin/env bash
+manage_session() {
+    printf 'session\n'
+}
DIFF

    run bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    printf '%s\n' "$output" | grep -q '^NEW_ABSTRACTION_SINGLE_CALLER:'
}

@test "NEW_ABSTRACTION_SINGLE_CALLER: new file without pattern name emits nothing" {
    diff="$TMPDIR_BATS/plain_add.diff"
    cat > "$diff" <<'DIFF'
diff --git a/scripts/helpers.sh b/scripts/helpers.sh
new file mode 100644
--- /dev/null
+++ b/scripts/helpers.sh
@@ -0,0 +1,3 @@
+#!/usr/bin/env bash
+do_thing() { printf 'ok\n'; }
DIFF

    run bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    count="$(printf '%s\n' "$output" | grep -c '^NEW_ABSTRACTION_SINGLE_CALLER:' || true)"
    [ "$count" -eq 0 ]
}

@test "NEW_ABSTRACTION_SINGLE_CALLER: linter:allow suppresses finding" {
    diff="$TMPDIR_BATS/manager_allowed.diff"
    cat > "$diff" <<'DIFF'
diff --git a/scripts/lock-manager.sh b/scripts/lock-manager.sh
new file mode 100644
--- /dev/null
+++ b/scripts/lock-manager.sh
@@ -0,0 +1,5 @@
+#!/usr/bin/env bash
+# linter:allow-NEW_ABSTRACTION_SINGLE_CALLER planned second caller in issue #999
+manage_lock() {
+    printf 'lock\n'
+}
DIFF

    run bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    count="$(printf '%s\n' "$output" | grep -c '^NEW_ABSTRACTION_SINGLE_CALLER:' || true)"
    [ "$count" -eq 0 ]
}

# ---------------------------------------------------------------------------
# Clean diff — all three silent
# ---------------------------------------------------------------------------

@test "clean reuse-correct diff emits no reuse-triage findings" {
    diff="$TMPDIR_BATS/clean.diff"
    cat > "$diff" <<'DIFF'
diff --git a/scripts/greet.sh b/scripts/greet.sh
new file mode 100644
--- /dev/null
+++ b/scripts/greet.sh
@@ -0,0 +1,3 @@
+#!/usr/bin/env bash
+say_hello() { printf 'hello\n'; }
DIFF

    run bash "$LINT" --diff-file "$diff"
    echo "# output: $output" >&3
    for rule in REINVENT_REPO_UTIL NEW_DEP_UNJUSTIFIED NEW_ABSTRACTION_SINGLE_CALLER; do
        count="$(printf '%s\n' "$output" | grep -c "^${rule}:" || true)"
        [ "$count" -eq 0 ]
    done
}

#!/usr/bin/env bats
# tests/simplify-guard.bats — tests for scripts/simplify-guard.sh.
#
# A simplification diff must not add a file, must not be net-additive, and
# must not touch a path absent from the base diff. Real files, no mocks.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    GUARD="$REPO_ROOT/scripts/simplify-guard.sh"
    WORK="$(mktemp -d -t simplify-guard-test.XXXXXX)"
    BASE="$WORK/base.diff"
    SIMP="$WORK/simplify.diff"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

# write_base — base diff that rewrites src/app.py (one hunk, net-deleting).
write_base() {
    cat > "$BASE" <<'EOF'
diff --git a/src/app.py b/src/app.py
index 1234567..89abcde 100644
--- a/src/app.py
+++ b/src/app.py
@@ -1,10 +1,6 @@
 def main():
-    x = 1
-    y = 2
-    z = 3
-    w = 4
-    v = 5
     return x + y
EOF
}

# run_guard — invoke the guard on the fixture diffs.
run_guard() { bash "$GUARD" --base-diff "$BASE" --simplify-diff "$SIMP"; }

# ── Syntax / invocation ───────────────────────────────────────────────────────

@test "simplify-guard.sh: bash -n syntax check" {
    run bash -n "$GUARD"
    [ "$status" -eq 0 ]
}

@test "simplify-guard.sh: --help exits 0" {
    run bash "$GUARD" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage: scripts/simplify-guard.sh"* ]]
}

@test "simplify-guard.sh: missing --simplify-diff file exits 2" {
    write_base
    rm -f "$SIMP"
    run run_guard
    [ "$status" -eq 2 ]
}

@test "simplify-guard.sh: missing --base-diff file exits 2" {
    printf -- '--- a/x\n+++ b/x\n' > "$SIMP"
    rm -f "$BASE"
    run run_guard
    [ "$status" -eq 2 ]
}

@test "simplify-guard.sh: missing arguments exits 2" {
    run bash "$GUARD"
    [ "$status" -eq 2 ]
}

# ── Clean cases (exit 0, no stdout) ───────────────────────────────────────────

@test "empty simplify diff exits 0" {
    write_base
    : > "$SIMP"
    run run_guard
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "subset-path diff with deletions exceeding additions exits 0, no stdout" {
    write_base
    cat > "$SIMP" <<'EOF'
diff --git a/src/app.py b/src/app.py
index 89abcde..0001111 100644
--- a/src/app.py
+++ b/src/app.py
@@ -1,6 +1,4 @@
 def main():
-    x = 1
-    y = 2
     return x + y
EOF
    run run_guard
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "subset-path diff with deletions equal to additions exits 0" {
    write_base
    cat > "$SIMP" <<'EOF'
diff --git a/src/app.py b/src/app.py
index 89abcde..0001111 100644
--- a/src/app.py
+++ b/src/app.py
@@ -1,6 +1,5 @@
 def main():
-    x = 1
-    y = 2
-    return x + y
+    return 3
EOF
    run run_guard
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

# ── Violations (exit 1, SIMPLIFY_GUARD lines) ─────────────────────────────────

@test "diff containing new file mode exits 1" {
    write_base
    cat > "$SIMP" <<'EOF'
diff --git a/src/app.py b/src/app.py
index 89abcde..0001111 100644
--- a/src/app.py
+++ b/src/app.py
@@ -1,6 +1,4 @@
 def main():
-    x = 1
-    y = 2
     return x + y
diff --git a/src/helper.py b/src/helper.py
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/src/helper.py
@@ -0,0 +1,2 @@
+def helper():
+    return 0
EOF
    run run_guard
    [ "$status" -eq 1 ]
    [[ "$output" == *SIMPLIFY_GUARD:*new\ file* ]]
    grep -Eq '^SIMPLIFY_GUARD:[^:]*:[^:]*: ' <<< "$output"
}

@test "diff touching a path absent from the base diff exits 1" {
    write_base
    cat > "$SIMP" <<'EOF'
diff --git a/README.md b/README.md
index 1112222..3334444 100644
--- a/README.md
+++ b/README.md
@@ -1,3 +1,2 @@
 title
-removed line
 kept line
EOF
    run run_guard
    [ "$status" -eq 1 ]
    [[ "$output" == *README.md* ]]
    [[ "$output" == *absent* ]]
    grep -Eq '^SIMPLIFY_GUARD:[^:]*:[^:]*: ' <<< "$output"
}

@test "diff with more added than deleted lines exits 1" {
    write_base
    cat > "$SIMP" <<'EOF'
diff --git a/src/app.py b/src/app.py
index 89abcde..0001111 100644
--- a/src/app.py
+++ b/src/app.py
@@ -1,6 +1,7 @@
 def main():
-    x = 1
+    a = 1
+    b = 2
+    c = 3
     return x + y
EOF
    run run_guard
    [ "$status" -eq 1 ]
    [[ "$output" == *net-additive* ]]
    grep -Eq '^SIMPLIFY_GUARD:[^:]*:[^:]*: ' <<< "$output"
}

@test "each violation line matches the SIMPLIFY_GUARD grammar" {
    write_base
    cat > "$SIMP" <<'EOF'
diff --git a/src/app.py b/src/app.py
index 89abcde..0001111 100644
--- a/src/app.py
+++ b/src/app.py
@@ -1,6 +1,5 @@
 def main():
-    x = 1
+    a = 1
+    b = 2
     return x + y
diff --git a/stray.py b/stray.py
new file mode 100644
index 0000000..2222222
--- /dev/null
+++ b/stray.py
@@ -0,0 +1,1 @@
+print(1)
EOF
    run run_guard
    [ "$status" -eq 1 ]
    # Every non-blank stdout line matches the grammar; nothing else is printed.
    local re='^SIMPLIFY_GUARD:[^:]*:[^:]*: '
    while IFS= read -r line; do
        [[ -z "$line" ]] && continue
        [[ "$line" =~ $re ]] || false
    done <<< "$output"
    [ "$(grep -c . <<< "$output")" -ge 2 ]
}

@test "one SIMPLIFY_GUARD line per violation" {
    write_base
    cat > "$SIMP" <<'EOF'
diff --git a/one.py b/one.py
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/one.py
@@ -0,0 +1,1 @@
+a = 1
diff --git a/two.py b/two.py
new file mode 100644
index 0000000..2222222
--- /dev/null
+++ b/two.py
@@ -0,0 +1,1 @@
+b = 2
EOF
    run run_guard
    [ "$status" -eq 1 ]
    [ "$(grep -c '^SIMPLIFY_GUARD:' <<< "$output")" -eq 5 ]  # 2 absent paths + 2 new files + net-additive
}

#!/usr/bin/env bats
# tests/self-enforce-qa.bats — TDD for scripts/self-enforce-qa.sh (issue #419)

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/self-enforce-qa.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures/self-enforce"
REAL_REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/.." && pwd)"

# self-enforce-qa.sh's QA chain runs the REAL ~13-minute `autospec validate`
# as step 2 (self-enforce-qa.sh:131). Every test that invokes the chain against
# a diff therefore took ~13min EACH — running unbounded under a directory sweep
# (`bats -r`, CI) it looks like a hang and freezes the runner (epic #1280 bats
# hang sweep). This helper builds a fast stub REPO_ROOT that keeps the REAL
# lint-implementation.sh (so the deterministic SECURITY/lint detection these
# tests assert is still genuinely exercised) but swaps validate.sh for a trivial
# exit-0 stub. The lint step is what actually decides the bad/good verdict here;
# validate.sh on a synthetic diff only added latency, not signal.
_fast_repo_root() {
  local d
  d="$(mktemp -d -t self-enforce-root-XXXXXX)"
  mkdir -p "$d/scripts"
  cp "$REAL_REPO_ROOT/scripts/lint-implementation.sh" "$d/scripts/lint-implementation.sh"
  printf '#!/usr/bin/env bash\nexit 0\n' > "$d/autospec validate"
  chmod +x "$d/autospec validate"
  printf '%s' "$d"
}

setup() {
  mkdir -p "$FIXTURES_DIR/bad"
  mkdir -p "$FIXTURES_DIR/good"

  # Bad fixture: contains a SECURITY violation (eval call)
  cat > "$FIXTURES_DIR/bad/bad-script.sh" <<'EOF'
#!/usr/bin/env bash
# This script has a SECURITY violation
eval "$USER_INPUT"
echo "done"
EOF
  chmod +x "$FIXTURES_DIR/bad/bad-script.sh"

  # Good fixture: clean script with no violations
  cat > "$FIXTURES_DIR/good/good-script.sh" <<'EOF'
#!/usr/bin/env bash
# This is a clean script
set -eu
echo "hello world"
EOF
  chmod +x "$FIXTURES_DIR/good/good-script.sh"
}

teardown() {
  rm -rf "$FIXTURES_DIR"
}

@test "self-enforce-qa.sh is executable" {
  [ -x "$SCRIPT" ]
}

@test "self-enforce-qa.sh --help exits 0" {
  run "$SCRIPT" --help
  [ "$status" -eq 0 ]
}

@test "self-enforce-qa.sh exits 1 without required --diff-file" {
  run "$SCRIPT"
  [ "$status" -ne 0 ]
}

@test "self-enforce-qa.sh exits 1 if --diff-file does not exist" {
  run "$SCRIPT" --diff-file "/nonexistent/path.diff"
  [ "$status" -ne 0 ]
}

@test "self-enforce-qa.sh exits non-zero on bad fixture with SECURITY violation" {
  # Create a diff with a SECURITY violation (--no-verify bypasses git hooks)
  diff_file=$(mktemp -t self-enforce-bad-XXXXXX.diff)
  cat > "$diff_file" <<'EOF'
diff --git a/scripts/bad-script.sh b/scripts/bad-script.sh
new file mode 100755
index 0000000..abc1234
--- /dev/null
+++ b/scripts/bad-script.sh
@@ -0,0 +1,4 @@
+#!/usr/bin/env bash
+# This script has a SECURITY violation
+git commit --no-verify -m "bypass hooks"
+echo "done"
EOF
  export REPO_ROOT="$(_fast_repo_root)"
  run "$SCRIPT" --diff-file "$diff_file"
  rm -rf "$REPO_ROOT"; rm -f "$diff_file"
  [ "$status" -ne 0 ]
}

@test "self-enforce-qa.sh output contains finding description on bad fixture" {
  diff_file=$(mktemp -t self-enforce-bad-XXXXXX.diff)
  cat > "$diff_file" <<'EOF'
diff --git a/scripts/bad-script.sh b/scripts/bad-script.sh
new file mode 100755
index 0000000..abc1234
--- /dev/null
+++ b/scripts/bad-script.sh
@@ -0,0 +1,4 @@
+#!/usr/bin/env bash
+# bad script with hook bypass
+git commit --no-verify -m "bypass hooks"
+echo "done"
EOF
  export REPO_ROOT="$(_fast_repo_root)"
  run "$SCRIPT" --diff-file "$diff_file"
  rm -rf "$REPO_ROOT"; rm -f "$diff_file"
  [ "$status" -ne 0 ]
  printf '%s\n' "$output" | grep -qiE "SECURITY|no-verify|finding|violation|blocked"
}

@test "self-enforce-qa.sh exits 0 on clean fixture" {
  diff_file=$(mktemp -t self-enforce-good-XXXXXX.diff)
  cat > "$diff_file" <<'EOF'
diff --git a/scripts/good-script.sh b/scripts/good-script.sh
new file mode 100755
index 0000000..def5678
--- /dev/null
+++ b/scripts/good-script.sh
@@ -0,0 +1,4 @@
+#!/usr/bin/env bash
+# This is a clean script
+set -eu
+echo "hello world"
EOF
  export REPO_ROOT="$(_fast_repo_root)"
  run "$SCRIPT" --diff-file "$diff_file"
  rm -rf "$REPO_ROOT"; rm -f "$diff_file"
  [ "$status" -eq 0 ]
}

@test "self-enforce-qa.sh honors [skip self-enforce] token" {
  diff_file=$(mktemp -t self-enforce-bad-XXXXXX.diff)
  cat > "$diff_file" <<'EOF'
diff --git a/scripts/bad-script.sh b/scripts/bad-script.sh
new file mode 100755
index 0000000..abc1234
--- /dev/null
+++ b/scripts/bad-script.sh
@@ -0,0 +1,4 @@
+#!/usr/bin/env bash
+git commit --no-verify -m "bypass hooks"
+echo "done"
EOF
  run env SELF_ENFORCE_SKIP_TOKEN="[skip self-enforce]" \
    "$SCRIPT" --diff-file "$diff_file" --pr-body "[skip self-enforce]"
  rm -f "$diff_file"
  [ "$status" -eq 0 ]
}

# The autospec-self-enforce GitHub Actions workflow was deliberately removed
# in #481 (commit 21d4f52, "chore: disable GitHub Actions / CI") when CI was
# disabled repo-wide; local pre-commit hooks + on-demand autospec validate
# now carry self-enforcement. This asserts the workflow stays absent so the
# stale existence/trigger/path-filter assertions can never silently re-appear.
@test "autospec-self-enforce workflow is intentionally absent (CI disabled in #481)" {
  [ ! -f "${BATS_TEST_DIRNAME}/../.github/workflows/autospec-self-enforce.yml" ]
}

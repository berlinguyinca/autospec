#!/usr/bin/env bats
# tests/self-enforce-smoke.bats — adversarial smoke tests for the autospec
# self-enforcement QA chain (scripts/self-enforce-qa.sh).
#
# These tests spawn deliberately-bad fixture diffs and assert the chain catches
# blocking violations. They do NOT require a live GitHub PR.
#
# Cases:
#   1. EVAL_USER_INPUT — eval "$USER_INPUT" triggers SECURITY finding
#   2. NO_VERIFY_BYPASS — --no-verify flag triggers SECURITY finding
#   3. SKIP_TOKEN_ESCAPE — [skip self-enforce] in PR body → exits 0 despite violation

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/self-enforce-qa.sh"
REAL_REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/.." && pwd)"

setup() {
    TMP_DIR="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMP_DIR" "${FAST_ROOT:-}"
}

# self-enforce-qa.sh's QA chain runs the REAL ~13-minute autospec validate as
# step 2 (self-enforce-qa.sh:131), so cases 1-2 below — which invoke the chain
# against a bad diff — each took ~13min and froze any directory-sweeping runner
# (epic #1280 bats hang sweep). This stub REPO_ROOT keeps the REAL
# lint-implementation.sh (the deterministic detector these SECURITY assertions
# depend on) and swaps validate.sh for a fast exit-0 stub; the lint step is what
# produces the SECURITY verdict, so signal is preserved and the ~13min latency
# is removed. Case 3 (skip-token) bypasses the chain entirely and stays fast.
_fast_repo_root() {
    FAST_ROOT="$(mktemp -d -t self-enforce-smoke-root-XXXXXX)"
    mkdir -p "$FAST_ROOT/scripts"
    cp "$REAL_REPO_ROOT/scripts/lint-implementation.sh" "$FAST_ROOT/scripts/lint-implementation.sh"
    printf '#!/usr/bin/env bash\nexit 0\n' > "$FAST_ROOT/autospec validate"
    chmod +x "$FAST_ROOT/autospec validate"
    printf '%s' "$FAST_ROOT"
}

# ── Helper: build a minimal unified diff with the given content ───────────────
make_diff() {
    local filepath="$1"
    local content="$2"
    cat <<EOF
diff --git a/${filepath} b/${filepath}
new file mode 100755
index 0000000..abc1234
--- /dev/null
+++ b/${filepath}
@@ -0,0 +1,$(printf '%s' "$content" | wc -l | tr -d ' ') @@
$(printf '%s\n' "$content" | sed 's/^/+/')
EOF
}

# ── Case 1: eval( call → SECURITY finding, exit non-zero ─────────────────────
@test "smoke: eval() call detected as SECURITY finding" {
    DIFF_FILE="$TMP_DIR/eval-violation.diff"
    make_diff "scripts/bad-impl.sh" '#!/usr/bin/env bash
set -eu
# Process user request — SECURITY violation: eval( usage
result=$(eval("$USER_INPUT"))
echo "done"' > "$DIFF_FILE"

    REPO_ROOT="$(_fast_repo_root)" \
        run bash "$SCRIPT" --diff-file "$DIFF_FILE"

    # Must exit non-zero (blocking finding)
    [ "$status" -ne 0 ]
    # Output must mention SECURITY
    [[ "$output" == *"SECURITY"* ]]
}

# ── Case 2: --no-verify flag → SECURITY finding, exit non-zero ───────────────
@test "smoke: --no-verify bypass detected as SECURITY finding" {
    DIFF_FILE="$TMP_DIR/no-verify-violation.diff"
    make_diff "scripts/deploy.sh" '#!/usr/bin/env bash
set -eu
# Deploy step
git commit --no-verify -m "force commit"
git push origin main' > "$DIFF_FILE"

    REPO_ROOT="$(_fast_repo_root)" \
        run bash "$SCRIPT" --diff-file "$DIFF_FILE"

    # Must exit non-zero
    [ "$status" -ne 0 ]
    # Output must mention SECURITY
    [[ "$output" == *"SECURITY"* ]]
}

# ── Case 3: [skip self-enforce] in PR body → exits 0 despite violation ───────
@test "smoke: [skip self-enforce] bootstrap escape exits 0 despite violation" {
    DIFF_FILE="$TMP_DIR/eval-violation-skip.diff"
    make_diff "scripts/bad-impl.sh" '#!/usr/bin/env bash
set -eu
result=$(eval("$USER_INPUT"))' > "$DIFF_FILE"

    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)" \
        run bash "$SCRIPT" \
            --diff-file "$DIFF_FILE" \
            --pr-body "This PR intentionally introduces a violation.
[skip self-enforce]
Reason: adversarial fixture for testing."

    # Must exit 0 (skip token bypasses chain)
    [ "$status" -eq 0 ]
    # Output should acknowledge the skip
    [[ "$output" == *"skip"* ]]
}

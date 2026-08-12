#!/usr/bin/env bats
# tests/test_lint_complexity_gates.bats — regression for feat(lint) #805
#
# Verifies the four deterministic complexity gates added to lint-implementation.sh:
# check_file_loc, check_function_loc, check_cyclomatic, check_duplicate_names.

LINT_SH="${BATS_TEST_DIRNAME}/../scripts/lint-implementation.sh"

@test "lint-implementation.sh contains check_file_loc function" {
    run grep -c "^check_file_loc()" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "lint-implementation.sh contains check_function_loc function" {
    run grep -c "^check_function_loc()" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "lint-implementation.sh contains check_cyclomatic function" {
    run grep -c "^check_cyclomatic()" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "lint-implementation.sh contains check_duplicate_names function" {
    run grep -c "^check_duplicate_names()" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "AUTOSPEC_MAX_FILE_LOC threshold is configurable via env var" {
    run grep -c "AUTOSPEC_MAX_FILE_LOC" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "AUTOSPEC_MAX_FUNC_LOC threshold is configurable via env var" {
    run grep -c "AUTOSPEC_MAX_FUNC_LOC" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "AUTOSPEC_MAX_CYCLOMATIC threshold is configurable via env var" {
    run grep -c "AUTOSPEC_MAX_CYCLOMATIC" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "check_file_loc is wired into the main detector pass" {
    # Both invocation paths (directives and non-directives) must call check_file_loc.
    run grep -c "check_file_loc" "$LINT_SH"
    [ "$status" -eq 0 ]
    # At least definition + 2 call sites
    [ "$output" -ge 3 ]
}

@test "check_duplicate_names is wired into the main detector pass" {
    run grep -c "check_duplicate_names" "$LINT_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 3 ]
}

# ─────────────────────────────────────────────────────────────────
# Issue #1245 regressions: AST-based Python nesting + setUp exemption
# ─────────────────────────────────────────────────────────────────

setup() {
    WORK="$(mktemp -d -t lint-complexity-1245.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

# make_added_file_diff SRC_FILE DIFF_PATH OUT_DIFF
# Build a "whole new file" unified diff for SRC_FILE under repo path DIFF_PATH.
make_added_file_diff() {
    local src="$1" path="$2" out="$3"
    local n
    n="$(wc -l < "$src" | tr -d ' ')"
    {
        printf 'diff --git a/%s b/%s\n' "$path" "$path"
        printf 'new file mode 100644\n'
        printf -- '--- /dev/null\n'
        printf '+++ b/%s\n' "$path"
        printf '@@ -0,0 +1,%s @@\n' "$n"
        sed 's/^/+/' "$src"
    } > "$out"
}

# Issue #1245 #1 false positive: docstring + multi-line call + dict literal,
# but SHALLOW real control-flow nesting (<=2). Must NOT emit a nesting finding.
@test "#1245: docstring/multiline-call/dict literal with shallow nesting → no nesting finding" {
    cat > "$WORK/stage_metadata.py" << 'PYEOF'
"""Module docstring that is long.

This docstring spans many indented lines so the indentation proxy would
    count these wrapped continuation lines
        and these
            and these
                and these
                    and these
                        as deep code nesting, which is wrong.
"""
import argparse


def build_parser():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--name",
                        type=str,
                        default="value",
                        help="a very long help string that wraps across "
                             "several continuation lines indented for readability",
    )
    config = {
        "a": {
            "b": {
                "c": {
                    "d": {
                        "e": 1,
                    },
                },
            },
        },
    }
    if config:
        return parser
    return parser
PYEOF
    make_added_file_diff "$WORK/stage_metadata.py" "stages/stage_metadata.py" "$WORK/meta.diff"
    run bash "$LINT_SH" --diff-file "$WORK/meta.diff"
    ! printf '%s\n' "$output" | grep -qE "COMPLEXITY:stages/stage_metadata.py:[0-9-]+: nesting"
}

# Issue #1245: genuine >4 control-flow nesting must still flag (no over-correction).
@test "#1245: genuine deep control-flow nesting (>4) → nesting finding emitted" {
    cat > "$WORK/deep.py" << 'PYEOF'
def deeply_nested(items):
    for a in items:
        if a:
            for b in a:
                if b:
                    for c in b:
                        return c
    return None
PYEOF
    make_added_file_diff "$WORK/deep.py" "stages/deep.py" "$WORK/deep.diff"
    run bash "$LINT_SH" --diff-file "$WORK/deep.diff"
    printf '%s\n' "$output" | grep -qE "COMPLEXITY:stages/deep.py:[0-9-]+: nesting"
}

# Issue #1245: two TestCase classes each defining setUp → no duplicate-name finding.
@test "#1245: setUp across distinct TestCase classes → no duplicate-name finding" {
    mkdir -p "$WORK/repo/tests"
    cat > "$WORK/repo/tests/test_a.py" << 'PYEOF'
import unittest


class TestAlpha(unittest.TestCase):
    def setUp(self):
        self.x = 1

    def test_alpha(self):
        self.assertEqual(self.x, 1)


class TestBeta(unittest.TestCase):
    def setUp(self):
        self.y = 2

    def test_beta(self):
        self.assertEqual(self.y, 2)
PYEOF
    make_added_file_diff "$WORK/repo/tests/test_a.py" "tests/test_a.py" "$WORK/dup.diff"
    run bash -c "cd '$WORK/repo' && bash '$LINT_SH' --diff-file '$WORK/dup.diff'"
    ! printf '%s\n' "$output" | grep -q "duplicate function name 'setUp'"
}

# Issue #1245: a real accidental dup of a domain function name must STILL flag.
@test "#1245: duplicate domain function name across files → still flagged" {
    cat > "$WORK/mod_a.py" << 'PYEOF'
def do_the_thing(x):
    return x + 1
PYEOF
    cat > "$WORK/mod_b.py" << 'PYEOF'
def do_the_thing(y):
    return y - 1
PYEOF
    {
        printf 'diff --git a/src/mod_a.py b/src/mod_a.py\n'
        printf 'new file mode 100644\n'
        printf -- '--- /dev/null\n'
        printf '+++ b/src/mod_a.py\n'
        printf '@@ -0,0 +1,2 @@\n'
        sed 's/^/+/' "$WORK/mod_a.py"
        printf 'diff --git a/src/mod_b.py b/src/mod_b.py\n'
        printf 'new file mode 100644\n'
        printf -- '--- /dev/null\n'
        printf '+++ b/src/mod_b.py\n'
        printf '@@ -0,0 +1,2 @@\n'
        sed 's/^/+/' "$WORK/mod_b.py"
    } > "$WORK/dupdomain.diff"
    # The dup-name check reads files on disk; place them at the diff paths.
    mkdir -p "$WORK/repo/src"
    cp "$WORK/mod_a.py" "$WORK/repo/src/mod_a.py"
    cp "$WORK/mod_b.py" "$WORK/repo/src/mod_b.py"
    run bash -c "cd '$WORK/repo' && bash '$LINT_SH' --diff-file '$WORK/dupdomain.diff'"
    printf '%s\n' "$output" | grep -q "duplicate function name 'do_the_thing'"
}

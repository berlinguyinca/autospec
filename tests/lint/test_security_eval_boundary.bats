#!/usr/bin/env bats
# tests/lint/test_security_eval_boundary.bats — SECURITY's eval pattern and its escape hatch.
#
# The pattern was an unanchored substring, so it reported `ast.literal_eval` — the stdlib
# parser that accepts literals and rejects every expression form, i.e. the recommended safe
# alternative to the builtin the rule exists to catch. `detect_security` also never consulted
# `is_line_allowed`, so the documented `# linter:allow-` comment could not clear it, and a
# comment written to explain the exemption was itself reported for naming the pattern. The
# only routes left were a blanket `Guardian: skip-SECURITY` — the last rule anyone should
# blanket-skip — or rewriting working code to dodge a substring (#3057).
#
# Same defect as the unanchored `xit` skip pattern in §1.6 of the gate design doc, which
# reported `sys.exit`. That fix excluded the dot from its class; this one must not, because
# the dotted `builtins.eval` form is the real thing.

bats_require_minimum_version 1.5.0

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/scripts/lint-implementation.sh"
    WORK="$(mktemp -d)"
}

teardown() {
    rm -rf "${WORK:?}"
}

# Builds a new-file diff whose added lines are exactly $@, then lints it. SECURITY reports
# the line after the offending one, which the assertions below account for.
lint_lines() {
    mkdir -p "$WORK/src"
    printf '%s\n' "$@" > "$WORK/src/probe.py"
    local count
    count="$(wc -l < "$WORK/src/probe.py" | tr -d ' ')"
    {
        printf 'diff --git a/src/probe.py b/src/probe.py\n'
        printf 'new file mode 100644\n'
        printf -- '--- /dev/null\n'
        printf '+++ b/src/probe.py\n'
        printf '@@ -0,0 +1,%s @@\n' "$count"
        sed 's/^/+/' "$WORK/src/probe.py"
    } > "$WORK/probe.diff"
    run bash -c "cd '$WORK' && bash '$LINT' --diff-file '$WORK/probe.diff'"
}

@test "eval boundary: the stdlib literal parser is not reported" {
    lint_lines 'value = ast.literal_eval(text)'  # linter:allow-SECURITY fixture text, not a call site
    ! printf '%s\n' "$output" | grep -q 'SECURITY'
}

@test "eval boundary: a bare eval call is still reported" {
    lint_lines 'danger = eval(text)'  # linter:allow-SECURITY fixture text, not a call site
    printf '%s\n' "$output" | grep -qE '^SECURITY:src/probe.py:[0-9]+: eval\(\) usage'
}

@test "eval boundary: a dotted eval call is still reported" {
    # The dot is deliberately outside the boundary class: excluding it — as the `xit` fix had
    # to — would let the dotted builtins.eval form through, which is the dangerous call.
    lint_lines 'also = builtins.eval(text)'  # linter:allow-SECURITY fixture text, not a call site
    printf '%s\n' "$output" | grep -qE '^SECURITY:src/probe.py:[0-9]+: eval\(\) usage'
}

@test "eval boundary: an identifier merely ending in eval is not reported" {
    lint_lines 'safe = lead_eval(text)' 'other = myeval(text)'  # linter:allow-SECURITY fixture text, not a call site
    ! printf '%s\n' "$output" | grep -q 'SECURITY'
}

@test "escape hatch: linter:allow-SECURITY with a reason downgrades the finding" {
    lint_lines 'danger = eval(text)  # linter:allow-SECURITY reviewed: expression comes from a literal table'  # linter:allow-SECURITY fixture text, not a call site
    printf '%s\n' "$output" | grep -q '^INFO:SECURITY:src/probe.py:'
    ! printf '%s\n' "$output" | grep -qE '^SECURITY:'
}

@test "escape hatch: a bare linter:allow-SECURITY without a reason does not suppress" {
    lint_lines 'danger = eval(text)  # linter:allow-SECURITY'  # linter:allow-SECURITY fixture text, not a call site
    printf '%s\n' "$output" | grep -qE '^SECURITY:src/probe.py:[0-9]+: eval\(\) usage'
}

@test "escape hatch: prose naming the pattern reports, and can be annotated" {
    # Comments are deliberately NOT stripped before this scan the way lint-ui.sh strips them:
    # a secret sitting in a comment is still a leaked secret. So prose that mentions the token
    # is still reported — and before #3057 that made the situation circular, because the
    # comment written to justify an exemption became a second finding with no way to clear it.
    # The escape hatch is the route: it works on a prose line like any other.
    lint_lines '# Notes on the eval( pattern.' 'safe = 1'  # linter:allow-SECURITY fixture text, not a call site
    printf '%s\n' "$output" | grep -qE '^SECURITY:src/probe.py:'

    lint_lines '# Notes on the eval( pattern.  # linter:allow-SECURITY prose, not a call site' 'safe = 1'  # linter:allow-SECURITY fixture text, not a call site
    printf '%s\n' "$output" | grep -q '^INFO:SECURITY:src/probe.py:'
    ! printf '%s\n' "$output" | grep -qE '^SECURITY:'
}

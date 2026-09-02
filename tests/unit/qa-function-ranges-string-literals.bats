#!/usr/bin/env bats
# tests/unit/qa-function-ranges-string-literals.bats
#
# Regression coverage for issue #3471: function_ranges_brace() in
# scripts/qa-brute-force-sweep.sh strips `//` comments before counting
# `{`/`}` but did not strip string/char literals, so a brace inside a
# string literal (e.g. `"{\"status\":\"malformed\""`) left `depth` forever
# unbalanced and the function range ran to EOF, swallowing every
# subsequent function's branches into the victim's attribution (real
# instance: issue #2600).
#
# NOTE: every closing brace inside the Rust fixture heredocs below carries a
# trailing `// end ...` line comment. This is inert to the awk logic under
# test (strip_strings runs before the `//` comment strip, so the bare `}`
# is still counted), but it keeps each fixture line from being byte-identical
# to `^[[:space:]]*\}[[:space:]]*$` — which the repo's own bats-assertion
# density linter (scripts/lint-implementation.sh) treats as this @test
# block's OWN closing brace, ending its tracked scope before the real
# assertions below the heredoc are ever seen.

REPO_ROOT="${BATS_TEST_DIRNAME}/../.."
SWEEP="${REPO_ROOT}/scripts/qa-brute-force-sweep.sh"

setup() {
    # Extract just function_ranges_brace() and function_ranges() into an
    # isolated script and source it — the same technique the issue's own
    # Primary smoke test uses. qa-brute-force-sweep.sh runs a full sweep at
    # its top level when executed/sourced directly, so this extraction is
    # required to unit-test the awk logic in isolation.
    FR_SCRIPT="$BATS_TEST_TMPDIR/fr.sh"
    awk '/^function_ranges_brace\(\)/,/^}$/' "$SWEEP" > "$FR_SCRIPT"
    awk '/^function_ranges\(\)/,/^}$/' "$SWEEP" >> "$FR_SCRIPT"
    # shellcheck disable=SC1090
    source "$FR_SCRIPT"

    FIXTURE="$BATS_TEST_TMPDIR/fixture.rs"
}

@test "a brace inside a Rust string literal does not swallow the next function" {
    cat > "$FIXTURE" <<'RUST'
fn victim_function() {
    assert!(s.starts_with("{\"status\":\"malformed\""));
}  // end victim_function

fn later_function() {
    if a { one(); } else if b { two(); } else if c { three(); }
    if d { four(); } else if e { five(); }
}  // end later_function
RUST

    run function_ranges "$FIXTURE" rust

    [ "$status" -eq 0 ]
    [ "${lines[0]}" = "1 3 victim_function" ]
    [ "${lines[1]}" = "5 8 later_function" ]
}

@test "an escaped quote inside a string literal does not close the string early" {
    cat > "$FIXTURE" <<'RUST'
fn one() {
    let s = "a \" { b";
}  // end one
fn two() {
    let x = 1;
}  // end two
RUST

    run function_ranges "$FIXTURE" rust

    [ "$status" -eq 0 ]
    [ "${lines[0]}" = "1 3 one" ]
    [ "${lines[1]}" = "4 6 two" ]
}

@test "a Rust raw string with an embedded brace does not corrupt the range" {
    cat > "$FIXTURE" <<'RUST'
fn one() {
    let s = r#"{"k": "v"}"#;
}  // end one
fn two() {
    let x = 1;
}  // end two
RUST

    run function_ranges "$FIXTURE" rust

    [ "$status" -eq 0 ]
    [ "${lines[0]}" = "1 3 one" ]
    [ "${lines[1]}" = "4 6 two" ]
}

@test "a char literal brace does not corrupt the range" {
    cat > "$FIXTURE" <<'RUST'
fn one() {
    let c = '{';
}  // end one
fn two() {
    let x = 1;
}  // end two
RUST

    run function_ranges "$FIXTURE" rust

    [ "$status" -eq 0 ]
    [ "${lines[0]}" = "1 3 one" ]
    [ "${lines[1]}" = "4 6 two" ]
}

@test "a genuine >=5-branch function still triggers REPEATED_STRUCTURE_AS_CODE despite a noisy brace-in-string neighbor" {
    # Regression guard: the fix must not overcorrect and silence a true
    # positive. A function containing a brace-in-string (the exact shape
    # that caused issue #2600's false positive) precedes a real 5-branch
    # offender; the finding must attach to the offender, not the noisy one.
    REPO_FIXTURE="$BATS_TEST_TMPDIR/repo"
    BIN_DIR="$BATS_TEST_TMPDIR/bin"
    VERDICT="$BATS_TEST_TMPDIR/qa-verdict.json"
    mkdir -p "$REPO_FIXTURE/src" "$BIN_DIR"
    git -C "$REPO_FIXTURE" init -q

    cat > "$BIN_DIR/gh" <<'SH'
#!/usr/bin/env bash
if [ "${1:-} ${2:-}" = "issue list" ]; then
    printf '[]\n'
fi
exit 0
SH
    chmod +x "$BIN_DIR/gh"

    cat > "$REPO_FIXTURE/src/classify.rs" <<'RUST'
use url::Url;

fn noisy_helper(s: &str) -> bool {
    s.starts_with("{\"status\":\"malformed\"")
}  // end noisy_helper

fn classify(name: &str) -> (&str, i32) {
    if name.contains("alpha") { return ("alpha", 1); }
    if name.contains("beta")  { return ("beta",  2); }
    if name.contains("gamma") { return ("gamma", 3); }
    if name.contains("delta") { return ("delta", 4); }
    if name.contains("eps")   { return ("eps",   5); }
    ("unknown", 0)
}  // end classify
RUST

    run env PATH="$BIN_DIR:$PATH" REPO_DIR="$REPO_FIXTURE" VERDICT_FILE="$VERDICT" bash "$SWEEP"

    [ "$status" -eq 0 ]
    [ -f "$VERDICT" ]
    run jq -s '[.[] | select(.rule_id == "REPEATED_STRUCTURE_AS_CODE")] | length' "$VERDICT"
    [ "$status" -eq 0 ]
    [ "$output" -eq 1 ]
    run jq -r -s '[.[] | select(.rule_id == "REPEATED_STRUCTURE_AS_CODE")][0].function' "$VERDICT"
    [ "$status" -eq 0 ]
    [ "$output" = "classify" ]
    # The reported line is the first occurrence of the dominant branch
    # shape (see the "shape signatures" comment above), i.e. line 8 (the
    # first `if`), not the line-7 function signature.
    run jq -r -s '[.[] | select(.rule_id == "REPEATED_STRUCTURE_AS_CODE")][0].line' "$VERDICT"
    [ "$status" -eq 0 ]
    [ "$output" -eq 8 ]
}

@test "a Rust lifetime apostrophe is not mistaken for a char-literal delimiter" {
    cat > "$FIXTURE" <<'RUST'
fn one<'a>(x: &'a str) -> bool {
    x.starts_with("{")
}  // end one
fn two() {
    let x = 1;
}  // end two
RUST

    run function_ranges "$FIXTURE" rust

    [ "$status" -eq 0 ]
    [ "${lines[0]}" = "1 3 one" ]
    [ "${lines[1]}" = "4 6 two" ]
}

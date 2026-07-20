#!/usr/bin/env bats

setup() {
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

    SWEEP="$BATS_TEST_DIRNAME/../../scripts/qa-brute-force-sweep.sh"
    export REPO_FIXTURE BIN_DIR VERDICT SWEEP
}

run_sweep() {
    rm -f "$VERDICT"
    run env PATH="$BIN_DIR:$PATH" REPO_DIR="$REPO_FIXTURE" VERDICT_FILE="$VERDICT" bash "$SWEEP"
}

rule_count() {
    local rule_id="$1"
    if [ ! -f "$VERDICT" ]; then
        printf '0\n'
        return
    fi
    jq -s --arg rule_id "$rule_id" '[.[] | select(.rule_id == $rule_id)] | length' "$VERDICT"
}

@test "Rust output assertions do not count as string-match domain logic" {
    cat > "$REPO_FIXTURE/src/output.rs" <<'RUST'
use std::time::Duration;

fn verify_output(stdout: &str) {
    assert!(stdout.contains("created"));
    assert!(stdout.contains("updated"));
    assert!(stdout.contains("complete"));
    let _timeout = Duration::from_secs(1);
}
RUST

    run_sweep

    [ "$status" -eq 0 ]
    [ "$(rule_count STRING_MATCH_DOMAIN_LOGIC)" -eq 0 ]
}

@test "Rust control-flow substring checks still emit one string-match finding" {
    cat > "$REPO_FIXTURE/src/classify.rs" <<'RUST'
use serde::Deserialize;

fn classify(value: &str) -> u8 {
    if value.contains("alpha") { return 1; }
    if value.contains("beta") { return 2; }
    if value.contains("gamma") { return 3; }
    0
}
RUST

    run_sweep

    [ "$status" -eq 0 ]
    [ "$(rule_count STRING_MATCH_DOMAIN_LOGIC)" -eq 1 ]
    [ "$(jq -sr '[.[] | select(.rule_id == "STRING_MATCH_DOMAIN_LOGIC")][0].line' "$VERDICT")" -eq 4 ]
}

@test "mixed assertion expectation snapshot logging and comments do not inflate the threshold" {
    cat > "$REPO_FIXTURE/src/mixed.rs" <<'RUST'
use std::time::Duration;

fn classify(value: &str) -> u8 {
    if value.contains("alpha") { return 1; }
    if value.contains("beta") { return 2; }
    assert!(value.contains("assertion"));
    expect(value.contains("expectation"));
    assert_snapshot!(value.contains("snapshot"));
    println!("{}", value.contains("logging"));
    // value.contains("comment-only")
    let _timeout = Duration::from_secs(1);
    0
}
RUST

    run_sweep

    [ "$status" -eq 0 ]
    [ "$(rule_count STRING_MATCH_DOMAIN_LOGIC)" -eq 0 ]
}

@test "repeated-structure finding remains byte-identical" {
    cat > "$REPO_FIXTURE/src/repeated.rs" <<'RUST'
fn dispatch(value: i32) -> i32 {
    if value == 1 { return 1; }
    if value == 2 { return 2; }
    if value == 3 { return 3; }
    if value == 4 { return 4; }
    if value == 5 { return 5; }
    0
}
RUST
    blob="$(git -C "$REPO_FIXTURE" hash-object src/repeated.rs)"

    run_sweep

    [ "$status" -eq 0 ]
    expected="{\"category\":\"code_health:brute_force_string_heuristics\",\"rule_id\":\"REPEATED_STRUCTURE_AS_CODE\",\"language\":\"rust\",\"file\":\"src/repeated.rs\",\"function\":\"dispatch\",\"scope\":\"dispatch\",\"line\":2,\"blob\":\"$blob\",\"filing_status\":\"created\",\"marker\":\"<!-- autospec-qa-brute-force:v1 rule=REPEATED_STRUCTURE_AS_CODE path=src/repeated.rs scope=dispatch blob=$blob -->\"}"
    [ "$(cat "$VERDICT")" = "$expected" ]
}

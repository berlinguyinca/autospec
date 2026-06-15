#!/usr/bin/env bats
# explore-learnings.bats — tests for explore-learnings.sh memo derivation.

SCRIPTS_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts"
LEDGER_SH="$SCRIPTS_DIR/explore-ledger.sh"
LEARN_SH="$SCRIPTS_DIR/explore-learnings.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    LEDGER="$TEST_TMP/l.jsonl"
    OUT="$TEST_TMP/explore-learnings.md"
    export AUTOSPEC_EXPLORE_LEDGER_BIN="$LEDGER_SH"
    BASH_BIN="$(command -v bash)"
}
teardown() {
    rm -rf "$TEST_TMP"
}

# Append a pending record. $1=source $2=title $3=issue $4=ts-suffix
_append() {
    local rec
    rec="$(printf '{"round":1,"source":"%s","title":"%s","norm_title":"%s","complexity":"small","confidence":0.8,"issue":%s,"pr":0,"outcome":"pending","reason":"","ts":"2026-06-14T00:00:0%sZ"}' \
        "$1" "$2" "$2" "$3" "$4")"
    bash "$LEDGER_SH" --ledger "$LEDGER" --append "$rec"
}

@test "learnings: source table has correct counts + clean-rate %" {
    _append spec-vs-code 'add login' 101 1
    _append spec-vs-code 'add logout' 102 2
    _append spec-vs-code 'add reset' 103 3
    _append internet 'add oauth' 104 4
    bash "$LEDGER_SH" --ledger "$LEDGER" --update-outcome 101 merged_clean ok
    bash "$LEDGER_SH" --ledger "$LEDGER" --update-outcome 102 merged_clean ok
    bash "$LEDGER_SH" --ledger "$LEDGER" --update-outcome 103 qa_failed bad
    bash "$LEDGER_SH" --ledger "$LEDGER" --update-outcome 104 abandoned nope

    run bash "$LEARN_SH" --ledger "$LEDGER" --out "$OUT"
    [ "$status" -eq 0 ]
    [ -f "$OUT" ]

    # spec-vs-code: filed 3, merged 2, failed 1, rate 67%
    grep -qE '\| spec-vs-code \| 3 \| 2 \| 1 \| 67% \|' "$OUT"
    # internet: filed 1, merged 0, failed 1, rate 0%
    grep -qE '\| internet \| 1 \| 0 \| 1 \| 0% \|' "$OUT"
    # spec-vs-code (67%) sorts before internet (0%)
    spec_line="$(grep -n 'spec-vs-code' "$OUT" | head -1 | cut -d: -f1)"
    inet_line="$(grep -n 'internet' "$OUT" | head -1 | cut -d: -f1)"
    [ "$spec_line" -lt "$inet_line" ]
}

@test "learnings: 'ships cleanly' lists a merged title" {
    _append spec-vs-code 'ship me clean' 201 1
    bash "$LEDGER_SH" --ledger "$LEDGER" --update-outcome 201 merged_clean ok
    run bash "$LEARN_SH" --ledger "$LEDGER" --out "$OUT"
    [ "$status" -eq 0 ]
    # Section present and the merged title listed under it.
    grep -q '## What ships cleanly here' "$OUT"
    grep -q -- '- ship me clean' "$OUT"
}

@test "learnings: 'failure modes' lists a failed title with reason" {
    _append internet 'flaky thing' 301 1
    bash "$LEDGER_SH" --ledger "$LEDGER" --update-outcome 301 qa_failed "tests flaked"
    run bash "$LEARN_SH" --ledger "$LEDGER" --out "$OUT"
    [ "$status" -eq 0 ]
    grep -q '## Recurring failure modes' "$OUT"
    grep -q 'flaky thing' "$OUT"
    grep -q 'tests flaked' "$OUT"
}

@test "learnings: empty ledger -> valid memo with 'no outcomes' note, exit 0" {
    run bash "$LEARN_SH" --ledger "$TEST_TMP/missing.jsonl" --out "$OUT"
    [ "$status" -eq 0 ]
    [ -f "$OUT" ]
    grep -q '# Explore Learnings' "$OUT"
    grep -qi 'no outcomes recorded yet' "$OUT"
    grep -q '## Generated:' "$OUT"
}

@test "learnings: absent ledger file is treated as empty" {
    run bash "$LEARN_SH" --ledger "$TEST_TMP/never-created.jsonl" --out "$OUT"
    [ "$status" -eq 0 ]
    grep -qi 'no outcomes recorded yet' "$OUT"
}

@test "learnings: jq missing -> exit 2" {
    # Empty PATH so command -v jq fails; invoke bash by absolute path.
    mkdir -p "$TEST_TMP/nojq"
    run env PATH="$TEST_TMP/nojq" "$BASH_BIN" "$LEARN_SH" --ledger "$LEDGER" --out "$OUT"
    [ "$status" -eq 2 ]
}

@test "learnings: -h prints usage, exit 0" {
    run bash "$LEARN_SH" -h
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'explore-learnings.sh'
}

@test "learnings: unknown arg -> exit 1" {
    run bash "$LEARN_SH" --bogus
    [ "$status" -eq 1 ]
}

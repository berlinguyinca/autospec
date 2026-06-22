#!/usr/bin/env bats
# explore-source-weights.bats — tests for explore-source-weights.sh.
#
# These build a REAL temp ledger via explore-ledger.sh --append so the
# stats -> weights path is exercised end to end (Task 1/2 integration), not
# a mock. Expected weights are pinned to the exact arithmetic from:
#   weight = (merged_clean + alpha * prior) / (filed + alpha)

SHARED_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
WEIGHTS_SH="$SHARED_DIR/scripts/explore-source-weights.sh"
LEDGER_SH="$SHARED_DIR/scripts/explore-ledger.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    LEDGER="$TEST_TMP/l.jsonl"
}
teardown() {
    rm -rf "$TEST_TMP"
}

# Append N records for a source, each with a distinct issue number and the
# given outcome. issue base lets callers keep sources from colliding.
# $1=source $2=count $3=outcome $4=issue_base
append_n() {
    local source="$1" count="$2" outcome="$3" base="$4" i issue rec
    for i in $(seq 1 "$count"); do
        issue=$((base + i))
        rec="$(printf '{"round":1,"source":"%s","title":"t%s","norm_title":"t%s","complexity":"small","confidence":0.8,"issue":%s,"pr":0,"outcome":"%s","reason":"","ts":"2026-06-14T00:00:00Z"}' \
            "$source" "$issue" "$issue" "$issue" "$outcome")"
        bash "$LEDGER_SH" --ledger "$LEDGER" --append "$rec"
    done
}

# Read a weight for a source out of the JSON output.
weight_of() { jq -r --arg s "$2" '.[$s]' <<<"$1"; }

# ── 1. absent ledger -> canonical priors ─────────────────────────────────────
@test "absent ledger -> canonical priors verbatim" {
    run bash "$WEIGHTS_SH" --ledger "$TEST_TMP/does-not-exist.jsonl"
    [ "$status" -eq 0 ]
    [ "$(weight_of "$output" spec-vs-code)" = "1.0" ] || [ "$(weight_of "$output" spec-vs-code)" = "1" ]
    [ "$(weight_of "$output" prior-reports)" = "0.9" ]
    [ "$(weight_of "$output" codebase-signals)" = "0.7" ]
    [ "$(weight_of "$output" open-issues)" = "0.6" ]
    [ "$(weight_of "$output" source-analysis)" = "0.5" ]
    [ "$(weight_of "$output" internet)" = "0.4" ]
}

# ── 2. empty ledger -> canonical priors ──────────────────────────────────────
@test "empty ledger -> canonical priors" {
    touch "$LEDGER"
    run bash "$WEIGHTS_SH" --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    [ "$(weight_of "$output" internet)" = "0.4" ]
    [ "$(weight_of "$output" prior-reports)" = "0.9" ]
}

# ── 3. spec-vs-code 10/10 clean + internet 0/10 (qa_failed), alpha=5 ──────────
@test "biased ledger: spec-vs-code 10/10 clean -> 1.0, internet 0/10 -> 0.1333" {
    append_n spec-vs-code 10 merged_clean 100
    append_n internet 10 qa_failed 200
    run bash "$WEIGHTS_SH" --ledger "$LEDGER" --alpha 5
    [ "$status" -eq 0 ]
    # (10 + 5*1.0)/(10+5) = 1.0
    sc="$(weight_of "$output" spec-vs-code)"
    [ "$sc" = "1.0" ] || [ "$sc" = "1" ]
    # (0 + 5*0.4)/(10+5) = 0.1333
    [ "$(weight_of "$output" internet)" = "0.1333" ]
}

# ── 4. prior 0.4, 10/10 clean, alpha=5 -> 0.8 ────────────────────────────────
@test "internet (prior 0.4) 10/10 clean alpha=5 -> 0.8" {
    append_n internet 10 merged_clean 300
    run bash "$WEIGHTS_SH" --ledger "$LEDGER" --alpha 5
    [ "$status" -eq 0 ]
    # (10 + 5*0.4)/(10+5) = 0.8
    [ "$(weight_of "$output" internet)" = "0.8" ]
}

# ── 5. alpha shrinks weight toward prior ─────────────────────────────────────
@test "higher alpha keeps weight closer to prior" {
    # internet prior 0.4, 10/10 clean.
    append_n internet 10 merged_clean 400
    run bash "$WEIGHTS_SH" --ledger "$LEDGER" --alpha 1
    [ "$status" -eq 0 ]
    w_lo="$(weight_of "$output" internet)"   # (10+1*0.4)/11 = 0.9455
    run bash "$WEIGHTS_SH" --ledger "$LEDGER" --alpha 20
    [ "$status" -eq 0 ]
    w_hi="$(weight_of "$output" internet)"   # (10+20*0.4)/30 = 0.6
    [ "$w_lo" = "0.9455" ]
    [ "$w_hi" = "0.6" ]
    # higher alpha (0.6) is closer to prior 0.4 than lower alpha (0.9455)
    closer="$(python3 -c "print(abs(0.6-0.4) < abs(0.9455-0.4))")"
    [ "$closer" = "True" ]
}

# ── 6. dependency-health is a canonical discovery source (prior 0.65) ─────────
@test "dependency-health (canonical) uses prior 0.65" {
    # dependency-health is in CANONICAL_SOURCES (explore-source-weights.sh), so
    # an absent ledger emits it at its canonical prior 0.65 (was previously a
    # non-canonical source defaulting to 0.5 / null; promoted in fix(explore)).
    run bash "$WEIGHTS_SH" --ledger "$TEST_TMP/missing.jsonl"
    [ "$status" -eq 0 ]
    [ "$(weight_of "$output" dependency-health)" = "0.65" ]

    # with ledger data: 3 filed, 0 clean (all qa_failed), alpha=5, prior 0.65,
    # plus the refute pull -> 0.4062 (script-rounded, pinned to live output).
    append_n dependency-health 3 qa_failed 500
    run bash "$WEIGHTS_SH" --ledger "$LEDGER" --alpha 5
    [ "$status" -eq 0 ]
    [ "$(weight_of "$output" dependency-health)" = "0.4062" ]
}

# ── 7. jq missing -> exit 2 ──────────────────────────────────────────────────
@test "jq missing -> exit 2 (fail-closed)" {
    fakebin="$TEST_TMP/bin"
    mkdir -p "$fakebin"
    # PATH containing only python3/bash/coreutils dirs but no jq.
    cat > "$fakebin/run.sh" <<EOF
#!/usr/bin/env bash
export PATH="$fakebin"
exec bash "$WEIGHTS_SH" --ledger "$TEST_TMP/missing.jsonl"
EOF
    # Symlink bash and python3 (needed by the script) into fakebin, omit jq.
    ln -s "$(command -v bash)" "$fakebin/bash"
    ln -s "$(command -v python3)" "$fakebin/python3"
    ln -s "$(command -v dirname)" "$fakebin/dirname"
    ln -s "$(command -v cat)" "$fakebin/cat"
    chmod +x "$fakebin/run.sh"
    run bash "$fakebin/run.sh"
    [ "$status" -eq 2 ]
}

# ── 8. output is valid JSON and all weights in [0,1] ─────────────────────────
@test "output is valid JSON; every weight in [0,1]" {
    append_n spec-vs-code 4 merged_clean 600
    append_n internet 6 qa_failed 700
    run bash "$WEIGHTS_SH" --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.' >/dev/null
    bad="$(echo "$output" | jq -r 'to_entries[] | select(.value < 0 or .value > 1) | .key')"
    [ -z "$bad" ]
}

# ── 9. --explain prints per-source lines to stderr ───────────────────────────
@test "--explain emits per-source explanation on stderr" {
    append_n internet 10 merged_clean 800
    run bash -c "bash '$WEIGHTS_SH' --ledger '$LEDGER' --alpha 5 --explain 2>&1 1>/dev/null"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "internet: filed=10 clean=10 prior=0.4 -> weight=0.8"
}

#!/usr/bin/env bats
# tests/local-anchor-verification.bats — TDD for guardrail R8 (tracker #3344,
# issue #3351): verbatim-anchor verification for local paraphrase output.
#
# The failure being guarded: given a manual about a "persistent agent", a
# local model writes a config about a "persistence agent" — a silent
# one-word substitution with no tell. Every claim a local dispatch makes
# about source text must carry a file:line anchor AND a verbatim quote span,
# and the quote must exist verbatim at the named location or the claim is
# DROPPED, not escalated: the exit stays 0 so no human triages fabricated
# quotes. The drop count lands on the routing ledger as anchor_drops.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/local-dispatch.sh"
LEDGER="${BATS_TEST_DIRNAME}/../scripts/routing-ledger.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/local-anchor-verify-XXXXXX")"
    # A manual about a persistent agent. Near-miss tests substitute one word
    # in its lines; the file also contains the substituted phrase inside a
    # different sentence, so any check looser than the exact span would
    # wrongly keep a fabricated quote.
    printf '%s\n' \
        '# Persistent agent' \
        '' \
        'The persistent agent retains its conversation state across sessions.' \
        'A persistence agent would be a different thing entirely.' \
        '' \
        'Config keys are listed in appendix A.' > "$TMP/manual.md"
    LEDGER_FILE="$TMP/routing-ledger.jsonl"
}

teardown() { rm -rf "$TMP"; }

# run_verify <claims-json> — run the checker with --root $TMP, expecting exit
# 0 (bats runs the body under set -e, so a non-zero exit fails the test).
# stdout -> $OUT_STD, stderr -> $TMP/err.
run_verify() {
    printf '%s' "$1" > "$TMP/claims.json"
    OUT_STD="$(bash "$SCRIPT" --verify-anchors "$TMP/claims.json" --root "$TMP" 2> "$TMP/err")"
}

# rec <id> [anchor_drops] — a minimal valid routing-ledger record.
rec() {
    if [ -n "${2:-}" ]; then
        printf '{"dispatch_id":"%s","ts":"2026-08-05T10:00:00Z","dispatch_kind":"implementer","profile":"qwen-local","model":"qwen3:32b","harness":"codex","issue":1,"cell_ctx":"64k","cell_reasoning":"deep","input_tokens":1000,"output_tokens":50,"cached_tokens":0,"wall_clock_ms":5000,"retries":0,"escalated":false,"anchor_drops":%s,"outcome":"merged_clean","reason":""}' "$1" "$2"
    else
        printf '{"dispatch_id":"%s","ts":"2026-08-05T10:00:00Z","dispatch_kind":"implementer","profile":"qwen-local","model":"qwen3:32b","harness":"codex","issue":1,"cell_ctx":"64k","cell_reasoning":"deep","input_tokens":1000,"output_tokens":50,"cached_tokens":0,"wall_clock_ms":5000,"retries":0,"escalated":false,"outcome":"merged_clean","reason":""}' "$1"
    fi
}

# ── the happy path ────────────────────────────────────────────────────────────

@test "a claim with a matching verbatim anchor passes" {
    run_verify '[{"claim":"state retention","anchor":"manual.md:3","quote":"The persistent agent retains its conversation state across sessions."}]'
    [ "$(printf '%s' "$OUT_STD" | jq 'length')" -eq 1 ]
    [ "$(printf '%s' "$OUT_STD" | jq -r '.[0].quote')" = "The persistent agent retains its conversation state across sessions." ]
    grep -q 'kept=1 dropped=0 total=1' "$TMP/err"
}

@test "a file-only anchor passes when the quote exists in the file" {
    run_verify '[{"claim":"appendix","anchor":"manual.md","quote":"Config keys are listed in appendix A."}]'
    [ "$(printf '%s' "$OUT_STD" | jq 'length')" -eq 1 ]
}

@test "claims are read from stdin via -" {
    OUT_STD="$(printf '%s' '[{"claim":"stdin","anchor":"manual.md:6","quote":"Config keys are listed in appendix A."}]' \
        | bash "$SCRIPT" --root "$TMP" --verify-anchors - 2> "$TMP/err")"
    [ "$(printf '%s' "$OUT_STD" | jq 'length')" -eq 1 ]
}

# ── the negative paths: fabricated and near-miss quotes are dropped ──────────

@test "a claim quoting text that is not in the source is dropped" {
    run_verify '[{"claim":"fabricated","anchor":"manual.md:3","quote":"The persistent agent deletes its history on restart."}]'
    [ "$(printf '%s' "$OUT_STD" | jq -c '.')" = "[]" ]
    grep -q 'kept=0 dropped=1 total=1' "$TMP/err"
    grep -q 'not verbatim' "$TMP/err"
}

@test "a one-word substitution in the quoted span is dropped" {
    # "persistent" -> "persistence": the whole point of R8. The file even
    # contains "A persistence agent" on line 4, so a looser match would pass.
    run_verify '[{"claim":"near-miss","anchor":"manual.md:3","quote":"The persistence agent retains its conversation state across sessions."}]'
    [ "$(printf '%s' "$OUT_STD" | jq -c '.')" = "[]" ]
    grep -q 'kept=0 dropped=1 total=1' "$TMP/err"
}

@test "a case-differing quote is not a verbatim match" {
    run_verify '[{"claim":"case","anchor":"manual.md:3","quote":"The PERSISTENT agent retains its conversation state across sessions."}]'
    [ "$(printf '%s' "$OUT_STD" | jq -c '.')" = "[]" ]
}

@test "a quote that exists on a different line than named is dropped" {
    run_verify '[{"claim":"wrong line","anchor":"manual.md:6","quote":"The persistent agent retains its conversation state across sessions."}]'
    [ "$(printf '%s' "$OUT_STD" | jq -c '.')" = "[]" ]
    grep -q 'not verbatim at manual.md:6' "$TMP/err"
}

@test "an anchor to a missing source is dropped" {
    run_verify '[{"claim":"missing file","anchor":"absent.md:1","quote":"anything"}]'
    [ "$(printf '%s' "$OUT_STD" | jq -c '.')" = "[]" ]
    grep -q 'source not found at absent.md' "$TMP/err"
}

@test "an unanchored claim is dropped without human escalation" {
    run_verify '[{"claim":"the agent persists state"}]'
    # exit 0 (set -e would have failed the test): dropped, not escalated.
    [ "$(printf '%s' "$OUT_STD" | jq -c '.')" = "[]" ]
    grep -q 'unanchored' "$TMP/err"
    grep -q 'kept=0 dropped=1 total=1' "$TMP/err"
}

@test "an anchor without a verbatim quote is dropped" {
    run_verify '[{"claim":"only a pointer","anchor":"manual.md:3"}]'
    [ "$(printf '%s' "$OUT_STD" | jq -c '.')" = "[]" ]
    grep -q 'unanchored' "$TMP/err"
}

@test "a mixed batch keeps only the verbatim claim and counts the drops" {
    run_verify '[
      {"claim":"good","anchor":"manual.md:3","quote":"The persistent agent retains its conversation state across sessions."},
      {"claim":"bad word","anchor":"manual.md:3","quote":"The persistence agent retains its conversation state across sessions."},
      {"claim":"no anchor"}
    ]'
    [ "$(printf '%s' "$OUT_STD" | jq 'length')" -eq 1 ]
    [ "$(printf '%s' "$OUT_STD" | jq -r '.[0].claim')" = "good" ]
    grep -q 'kept=1 dropped=2 total=3' "$TMP/err"
}

# ── argument surface ──────────────────────────────────────────────────────────

@test "a missing claims file is a usage error" {
    run bash "$SCRIPT" --verify-anchors "$TMP/nope.json" --root "$TMP"
    [ "$status" -eq 1 ]
    [[ "$output" == *"claims file not found"* ]]
}

@test "a missing root directory is a usage error" {
    printf '[]' > "$TMP/claims.json"
    run bash "$SCRIPT" --verify-anchors "$TMP/claims.json" --root "$TMP/absent"
    [ "$status" -eq 1 ]
    [[ "$output" == *"root directory not found"* ]]
}

@test "non-array claims input is a usage error" {
    printf '{"claim":"not","an":"array"}' > "$TMP/claims.json"
    run bash "$SCRIPT" --verify-anchors "$TMP/claims.json" --root "$TMP"
    [ "$status" -eq 1 ]
    [[ "$output" == *"JSON array"* ]]
}

@test "verify-anchors runs without the dispatch preconditions (no codex needed)" {
    printf '[{"claim":"ok","anchor":"manual.md:6","quote":"Config keys are listed in appendix A."}]' > "$TMP/claims.json"
    minbin="$TMP/minbin"
    mkdir -p "$minbin"
    for tool in bash jq grep sed cat; do
        resolved="$(command -v "$tool" 2>/dev/null || true)"
        if [ -n "$resolved" ]; then ln -sf "$resolved" "$minbin/$(basename "$resolved")"; fi
    done
    OUT_STD="$(env PATH="$minbin" bash "$SCRIPT" --verify-anchors "$TMP/claims.json" --root "$TMP" 2> "$TMP/err")"
    [ "$(printf '%s' "$OUT_STD" | jq 'length')" -eq 1 ]
}

# ── the drop count lands on the ledger ───────────────────────────────────────

@test "the drop count is recorded on the ledger row and surfaced in stats" {
    run_verify '[{"claim":"no anchor"},{"claim":"fabricated","anchor":"manual.md:3","quote":"wrong text"}]'
    _dropped="$(grep -o 'dropped=[0-9]*' "$TMP/err" | head -1 | cut -d= -f2)"
    [ "$_dropped" = "2" ]
    run bash "$LEDGER" --ledger "$LEDGER_FILE" --append "$(rec d1 "$_dropped")"
    [ "$status" -eq 0 ]
    # The row carries the count.
    run jq -r '.anchor_drops' "$LEDGER_FILE"
    [ "$output" = "2" ]
    # Stats total it per (kind, profile, cell).
    run bash "$LEDGER" --ledger "$LEDGER_FILE" --stats --json
    [ "$(printf '%s' "$output" | jq -r '.[0].anchor_drops')" = "2" ]
    # Text readers surface it too.
    run bash "$LEDGER" --ledger "$LEDGER_FILE" --show
    [[ "$output" == *"drops=2"* ]]
}

@test "a record without anchor_drops still appends (backward compatible)" {
    run bash "$LEDGER" --ledger "$LEDGER_FILE" --append "$(rec d1)"
    [ "$status" -eq 0 ]
    run bash "$LEDGER" --ledger "$LEDGER_FILE" --stats --json
    [ "$(printf '%s' "$output" | jq -r '.[0].anchor_drops')" = "0" ]
    run bash "$LEDGER" --ledger "$LEDGER_FILE" --show
    [[ "$output" == *"drops=0"* ]]
}

@test "anchor_drops must be a non-negative number" {
    run bash "$LEDGER" --ledger "$LEDGER_FILE" --append "$(rec d1 -1)"
    [ "$status" -eq 1 ]
    [[ "$output" == *"anchor_drops must be a non-negative number"* ]]
    run bash "$LEDGER" --ledger "$LEDGER_FILE" --append \
        '{"dispatch_id":"d1","ts":"t","dispatch_kind":"implementer","profile":"p","model":"m","harness":"h","issue":1,"cell_ctx":"64k","cell_reasoning":"deep","input_tokens":10,"output_tokens":10,"cached_tokens":0,"wall_clock_ms":1,"retries":0,"escalated":false,"anchor_drops":"2","outcome":"merged_clean","reason":""}'
    [ "$status" -eq 1 ]
    [[ "$output" == *"anchor_drops must be a non-negative number"* ]]
}

@test "an existing ledger row with anchor_drops still validates" {
    run bash "$LEDGER" --ledger "$LEDGER_FILE" --append "$(rec d1 3)"
    [ "$status" -eq 0 ]
    run bash "$LEDGER" --ledger "$LEDGER_FILE" --validate
    [ "$status" -eq 0 ]
}

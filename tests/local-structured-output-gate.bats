#!/usr/bin/env bats
# tests/local-structured-output-gate.bats — guardrail R7: a schema validator is
# a precondition for local structured-output routing (issue #3350, tracker
# #3344).
#
# Three contracts, all against the real scripts (no mocks):
#   1. scripts/local-structured-output.sh is the single declared mapping
#      dispatch kind -> output schema -> validator command, and it is the
#      default-deny gate route-decide.sh consults: a kind with a declared
#      schema but no registered validator is not local-eligible.
#   2. route-decide.sh applies the gate: structured kinds without a validator
#      fall through to the baseline, while kinds outside the mapping keep
#      their pre-R7 behavior exactly (parity).
#   3. `local-structured-output.sh --dispatch` wraps a local dispatch in
#      bounded retry (max 5) feeding the validator's findings back as
#      directives, escalates UP to the cloud fallback on exhaustion, and
#      records the retry count and outcome on the routing ledger.

GATE="${BATS_TEST_DIRNAME}/../scripts/local-structured-output.sh"
DECIDE="${BATS_TEST_DIRNAME}/../scripts/route-decide.sh"
LEDGER_SH="${BATS_TEST_DIRNAME}/../scripts/routing-ledger.sh"
SELECTOR="${BATS_TEST_DIRNAME}/../skills/autospec-run/scripts/select-model-profile.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/local-structured-output-gate-XXXXXX")"
    PROF="$TMP/profiles.yml"
    cat > "$PROF" <<'EOF'
claude-sonnet-cloud:
  model: claude-sonnet-4-6
  ctx: 120k
  reasoning: deep
  cost_in: 3.0
  cost_out: 15.0
qwen3-32b-laptop:
  model: qwen3:32b
  ctx: 120k
  reasoning: deep
  cost_minute: 0.02
EOF
}

teardown() { rm -rf "$TMP"; }

# stats_row <kind> <profile> <ctx> <reasoning> <dispatches> <first_pass>
#           <failure> <escalation> <mean_retries> <cache_hit>
# One aggregated ledger-stats row in the shape routing-cost.sh consumes
# (same shape as tests/route-decide-allowlist.bats).
stats_row() {
    printf '{"dispatch_kind":"%s","profile":"%s","cell_ctx":"%s","cell_reasoning":"%s","dispatches":%s,"first_pass_rate":%s,"failure_rate":%s,"escalation_rate":%s,"mean_retries":%s,"cache_hit_ratio":%s}' \
        "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "${10}"
}

# Strong local evidence + weak cloud evidence: the allowlisted control in
# tests/route-decide-allowlist.bats routes exactly these rows to the local
# model, so any test that ends on the baseline here was denied by a gate, not
# by cost.
write_evidence() {
    local kind="$1"
    jq -n --argjson a "$(stats_row "$kind" qwen3-32b-laptop 120k deep 50 0.95 0.02 0.02 0.1 0.0)" \
          --argjson b "$(stats_row "$kind" claude-sonnet-cloud 120k deep 50 0.60 0.20 0.20 1.0 0.0)" \
          '[$a,$b]' > "$TMP/s.json"
}

# rec <id> <kind> — a minimal valid ledger record (all REQUIRED_KEYS) that
# `--dispatch --record` merges the measured retry/outcome fields into.
rec() {
    printf '{"dispatch_id":"%s","ts":"2026-08-05T10:00:00Z","dispatch_kind":"%s","profile":"qwen3:32b","model":"qwen3:32b","harness":"codex-oss","issue":3350,"cell_ctx":"120k","cell_reasoning":"deep","input_tokens":1000,"output_tokens":10,"cached_tokens":0,"wall_clock_ms":100,"retries":0,"escalated":false,"outcome":"pending","reason":""}' \
        "$1" "$2"
}

# ── 1. the declared mapping ───────────────────────────────────────────────────

@test "the mapping declares exactly the two structured kinds, both without a registered validator" {
    run bash "$GATE" --kinds
    [ "$status" -eq 0 ]
    [ "$output" = "explore-researcher
qa-sweep" ]
    run bash "$GATE" --validator explore-researcher
    [ "$status" -eq 1 ]
    run bash "$GATE" --validator qa-sweep
    [ "$status" -eq 1 ]
}

@test "both mapped kinds are in the routing-ledger vocabulary" {
    allowed="$(sed -n 's/^ALLOWED_KINDS="\([^"]*\)".*/\1/p' "$LEDGER_SH" | head -n 1)"
    for k in explore-researcher qa-sweep; do
        _found=0
        for w in $allowed; do
            [ "$w" = "$k" ] && _found=1
        done
        [ "$_found" -eq 1 ]
    done
}

@test "--local-eligible: structured kind without validator is denied (rc 1)" {
    run bash "$GATE" --local-eligible qa-sweep
    [ "$status" -eq 1 ]
    [[ "$output" == *"no registered validator"* ]]
    run bash "$GATE" --local-eligible explore-researcher
    [ "$status" -eq 1 ]
}

@test "--local-eligible: kind with no declared schema is eligible (rc 0); empty kind is an error" {
    run bash "$GATE" --local-eligible implementer
    [ "$status" -eq 0 ]
    run bash "$GATE" --local-eligible refine-lens
    [ "$status" -eq 0 ]
    run bash "$GATE" --local-eligible ""
    [ "$status" -eq 1 ]
}

@test "registering a validator flips the kind local-eligible (mutation check)" {
    cp "$GATE" "$TMP/gate-mutated.sh"
    sed -i 's/^qa-sweep|findings-json|$/qa-sweep|findings-json|bash \/dev\/true/' "$TMP/gate-mutated.sh"
    run bash "$TMP/gate-mutated.sh" --local-eligible qa-sweep
    [ "$status" -eq 0 ]
    # The other declared kind is untouched by the mutation.
    run bash "$TMP/gate-mutated.sh" --local-eligible explore-researcher
    [ "$status" -eq 1 ]
    # And --validator now prints the registered command.
    run bash "$TMP/gate-mutated.sh" --validator qa-sweep
    [ "$status" -eq 0 ]
    [ "$output" = "bash /dev/true" ]
}

# ── 2. route-decide.sh applies the gate ───────────────────────────────────────

@test "route-decide: qa-sweep (no validator) falls through to the baseline despite strong local evidence" {
    write_evidence qa-sweep
    # Without --explain the default output is the winner alone: the baseline
    # cloud model, not the local one (the R7 gate stripped it).
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" \
        --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" \
        --kind qa-sweep
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
    # With --explain the gate announces itself on stderr.
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" \
        --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" \
        --kind qa-sweep --explain
    [ "$status" -eq 0 ]
    [[ "$output" == *"R7 gate: kind=qa-sweep declares a schema but no validator is registered"* ]]
}

@test "route-decide: a non-structured kind is unaffected by the gate (parity control)" {
    # Same evidence, kind outside the mapping: the pre-R7 routing stands and
    # the local model wins, so the qa-sweep denial above is the gate, not cost.
    write_evidence refine-lens
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" \
        --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" \
        --kind refine-lens
    [ "$status" -eq 0 ]
    [ "$output" = "qwen3:32b" ]
}

@test "route-decide: with a validator registered, qa-sweep routes local (full-scope control)" {
    # Full-scope positive control: a mutated copy of the gate with a validator
    # registered is the ONLY difference from the denied case above, so a local
    # winner here proves the gate is the deciding factor end to end.
    S="$TMP/scripts"
    mkdir -p "$S"
    cp "$DECIDE" "$S/route-decide.sh"
    cp "${BATS_TEST_DIRNAME}/../scripts/routing-cost.sh" "$S/"
    cp "$LEDGER_SH" "$S/routing-ledger.sh"
    cp "$SELECTOR" "$S/select-model-profile.sh"
    cp "$GATE" "$S/local-structured-output.sh"
    sed -i 's/^qa-sweep|findings-json|$/qa-sweep|findings-json|bash \/dev\/true/' "$S/local-structured-output.sh"
    write_evidence qa-sweep
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$S/route-decide.sh" --profiles-file "$PROF" \
        --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" \
        --kind qa-sweep
    [ "$status" -eq 0 ]
    [ "$output" = "qwen3:32b" ]
}

# ── 3. --dispatch: bounded retry, escalation, ledger record ──────────────────

# A local model that always emits an invalid payload and logs its state.
write_fake_local() {
    cat > "$TMP/local.sh" <<EOF
#!/usr/bin/env bash
printf 'attempt %s directives=%s\n' "\$(cat "$TMP/local-runs.log" | wc -l | tr -d ' ')" "\${AUTOSPEC_VALIDATION_DIRECTIVES:-}" >> "$TMP/local-runs.log"
printf 'attempt %s local model output\n' "\$(tail -n 1 "$TMP/local-runs.log" | cut -d' ' -f2)"
EOF
    chmod +x "$TMP/local.sh"
    : > "$TMP/local-runs.log"
}

@test "dispatch: every local attempt fails validation -> 6 local runs (1 + 5 retries), then exactly one cloud fallback; ledger row says escalated" {
    write_fake_local
    cat > "$TMP/val-fail.sh" <<'EOF'
#!/usr/bin/env bash
printf 'findings: required field %s missing\n' "'findings[]'" >&2
exit 1
EOF
    cat > "$TMP/fallback.sh" <<EOF
#!/usr/bin/env bash
printf 'cloud fallback ran %s\n' "\$(cat "$TMP/fb-runs.log" | wc -l | tr -d ' ')" >> "$TMP/fb-runs.log"
printf 'cloud result'
EOF
    chmod +x "$TMP/val-fail.sh" "$TMP/fallback.sh"
    : > "$TMP/fb-runs.log"
    LEDGER="$TMP/ledger.jsonl"
    run bash "$GATE" --dispatch qa-sweep \
        --local "bash" "$TMP/local.sh" \
        --validator-cmd "bash $TMP/val-fail.sh" \
        --fallback "bash" "$TMP/fallback.sh" \
        --record "$(rec d-exhaust qa-sweep)" --ledger "$LEDGER"
    # Escalation succeeded: the fallback's exit status is the wrapper's.
    [ "$status" -eq 0 ]
    [ "$(wc -l < "$TMP/local-runs.log" | tr -d ' ')" = "6" ]
    [ "$(wc -l < "$TMP/fb-runs.log" | tr -d ' ')" = "1" ]
    [[ "$output" == *"cloud result"* ]]
    [[ "$output" == *"escalating to cloud"* ]]
    # The validator's findings were fed back as directives on retry attempts.
    [[ "$output" == *"local attempt 2 failed schema validation"* ]]
    grep -q "directives=findings: required field" "$TMP/local-runs.log"
    # The first attempt ran WITHOUT stale directives.
    grep -q 'directives=$' <(head -n 1 "$TMP/local-runs.log")
    # The ledger row carries the retry count and the final outcome.
    run grep -c . "$LEDGER"
    [ "$output" = "1" ]
    run jq -c '.' "$LEDGER"
    [ "$status" -eq 0 ]
    row="$(head -n 1 "$LEDGER")"
    [ "$(printf '%s' "$row" | jq -r '.retries')" = "5" ]
    [ "$(printf '%s' "$row" | jq -r '.escalated')" = "true" ]
    [ "$(printf '%s' "$row" | jq -r '.outcome')" = "escalated" ]
    printf '%s' "$row" | jq -e '.reason | contains("escalated to cloud")' >/dev/null
    # The ledger row is schema-valid per routing-ledger.sh itself.
    run bash "$LEDGER_SH" --ledger "$LEDGER" --validate
    [ "$status" -eq 0 ]
}

@test "dispatch: local output valid on the 3rd attempt -> retried_ok, no escalation, findings fed back" {
    write_fake_local
    cat > "$TMP/val-fail2.sh" <<'EOF'
#!/usr/bin/env bash
n=0; [ -f "$FAKE_STATE/val-runs" ] && n="$(cat "$FAKE_STATE/val-runs")"
n=$((n + 1))
printf '%s' "$n" > "$FAKE_STATE/val-runs"
if [ "$n" -le 2 ]; then
    printf 'findings: attempt %s still invalid\n' "$n" >&2
    exit 1
fi
exit 0
EOF
    chmod +x "$TMP/val-fail2.sh"
    cat > "$TMP/fallback2.sh" <<EOF
#!/usr/bin/env bash
printf 'CLOUD MUST NOT RUN' >> "$TMP/fb-runs.log"
printf 'cloud result'
EOF
    chmod +x "$TMP/fallback2.sh"
    LEDGER="$TMP/ledger.jsonl"
    run env FAKE_STATE="$TMP" bash "$GATE" --dispatch qa-sweep \
        --local "bash" "$TMP/local.sh" \
        --validator-cmd "bash $TMP/val-fail2.sh" \
        --fallback "bash" "$TMP/fallback2.sh" \
        --record "$(rec d-retried qa-sweep)" --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    [ "$(wc -l < "$TMP/local-runs.log" | tr -d ' ')" = "3" ]
    [ ! -e "$TMP/fb-runs.log" ]
    # The attempt-2 findings reached attempt 3 verbatim as directives.
    grep -q "directives=findings: attempt 2 still invalid" "$TMP/local-runs.log"
    run jq -c '.' "$LEDGER"
    row="$(head -n 1 "$LEDGER")"
    [ "$(printf '%s' "$row" | jq -r '.retries')" = "2" ]
    [ "$(printf '%s' "$row" | jq -r '.escalated')" = "false" ]
    [ "$(printf '%s' "$row" | jq -r '.outcome')" = "retried_ok" ]
}

@test "dispatch: local output valid on the first attempt -> lgtm_first_pass, no retries, no escalation" {
    write_fake_local
    cat > "$TMP/val-ok.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$TMP/val-ok.sh"
    cat > "$TMP/fallback3.sh" <<EOF
#!/usr/bin/env bash
printf 'CLOUD MUST NOT RUN' >> "$TMP/fb-runs.log"
printf 'cloud result'
EOF
    chmod +x "$TMP/fallback3.sh"
    LEDGER="$TMP/ledger.jsonl"
    run bash "$GATE" --dispatch qa-sweep \
        --local "bash" "$TMP/local.sh" \
        --validator-cmd "bash $TMP/val-ok.sh" \
        --fallback "bash" "$TMP/fallback3.sh" \
        --record "$(rec d-firstpass qa-sweep)" --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    [ "$(wc -l < "$TMP/local-runs.log" | tr -d ' ')" = "1" ]
    [ ! -e "$TMP/fb-runs.log" ]
    row="$(head -n 1 "$LEDGER")"
    [ "$(printf '%s' "$row" | jq -r '.retries')" = "0" ]
    [ "$(printf '%s' "$row" | jq -r '.escalated')" = "false" ]
    [ "$(printf '%s' "$row" | jq -r '.outcome')" = "lgtm_first_pass" ]
}

@test "dispatch: refuses to wrap an unvalidated local dispatch (rc 4, nothing runs)" {
    write_fake_local
    cat > "$TMP/fallback4.sh" <<EOF
#!/usr/bin/env bash
printf 'CLOUD MUST NOT RUN' >> "$TMP/fb-runs.log"
printf 'cloud result'
EOF
    chmod +x "$TMP/fallback4.sh"
    run bash "$GATE" --dispatch qa-sweep \
        --local "bash" "$TMP/local.sh" \
        --fallback "bash" "$TMP/fallback4.sh"
    [ "$status" -eq 4 ]
    [ "$(wc -l < "$TMP/local-runs.log" | tr -d ' ')" = "0" ]
    [ ! -e "$TMP/fb-runs.log" ]
}

@test "dispatch: a kind outside the mapping is refused by the wrapper (rc 4)" {
    write_fake_local
    cat > "$TMP/fallback5.sh" <<EOF
#!/usr/bin/env bash
printf 'cloud result'
EOF
    chmod +x "$TMP/fallback5.sh"
    run bash "$GATE" --dispatch implementer \
        --local "bash" "$TMP/local.sh" \
        --validator-cmd "bash /dev/true" \
        --fallback "bash" "$TMP/fallback5.sh"
    [ "$status" -eq 4 ]
    [ "$(wc -l < "$TMP/local-runs.log" | tr -d ' ')" = "0" ]
}

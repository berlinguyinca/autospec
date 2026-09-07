#!/usr/bin/env bats
# tests/route-decide-allowlist.bats — pins the non-overridable dispatch-kind
# allowlist in scripts/route-decide.sh as an enforced invariant.
#
# Planning is the most expensive place to be cheap: operators report local
# models collapsing on repo-wide planning of complex codebases, while a bad
# decomposition costs N implementer cycles downstream. The allowlist therefore
# holds only the high-fan-out read-and-report kinds, and the planning-shaped
# kinds (spec-research, spec-design, broad-audit, refine-scope) must stay off
# it. A kind must not become overridable by omission or by a careless edit, so
# the exact contents of OVERRIDABLE_KINDS are pinned below: the pin fails on
# ANY growth or shrinkage, not just the kinds named here.

DECIDE="${BATS_TEST_DIRNAME}/../scripts/route-decide.sh"
LEDGER="${BATS_TEST_DIRNAME}/../scripts/routing-ledger.sh"
SELECTOR="${BATS_TEST_DIRNAME}/../skills/autospec-run/scripts/select-model-profile.sh"

# The frozen set, one kind per line, sorted.
EXPECTED_ALLOWLIST="explore-researcher
implementer
qa-sweep
refine-lens"

# The planning-shaped kinds: in the ledger vocabulary, never overridable.
PLANNING_KINDS="spec-research spec-design broad-audit refine-scope"

# _kinds <path> — echo the OVERRIDABLE_KINDS assignment of a route-decide.sh
# copy, one kind per line, sorted. Works on the real script and on test doubles.
_kinds() {
    sed -n 's/^OVERRIDABLE_KINDS="\([^"]*\)".*/\1/p' "$1" \
        | head -n 1 \
        | tr ' ' '\n' \
        | sort
}

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/route-decide-allowlist-XXXXXX")"
    EMPTY="$TMP/empty.json"
    printf '[]' > "$EMPTY"
    PROF="$TMP/profiles.yml"
    cat > "$PROF" <<'EOF'
claude-haiku-cloud:
  model: claude-haiku-4-5
  ctx: 64k
  reasoning: medium
  cost_in: 1.0
  cost_out: 5.0
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

# stats_row <kind> <profile> <ctx> <reasoning> <n> <fp> <fail> <esc> <retries> <cache>
stats_row() {
    printf '{"dispatch_kind":"%s","profile":"%s","cell_ctx":"%s","cell_reasoning":"%s","dispatches":%s,"first_pass_rate":%s,"failure_rate":%s,"escalation_rate":%s,"mean_retries":%s,"cache_hit_ratio":%s}' \
        "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "${10}"
}

# rec <id> <kind> — a minimal valid ledger record of the given dispatch_kind.
rec() {
    printf '{"dispatch_id":"%s","ts":"2026-08-05T10:00:00Z","dispatch_kind":"%s","profile":"p","model":"m","harness":"claude","issue":1,"cell_ctx":"64k","cell_reasoning":"deep","input_tokens":1000,"output_tokens":10,"cached_tokens":0,"wall_clock_ms":100,"retries":0,"escalated":false,"outcome":"merged_clean","reason":""}' \
        "$1" "$2"
}

# ── the pin: exact contents of OVERRIDABLE_KINDS ─────────────────────────────

@test "OVERRIDABLE_KINDS is exactly the four read-and-report kinds" {
    [ "$(_kinds "$DECIDE")" = "$EXPECTED_ALLOWLIST" ]
}

@test "no planning kind is on OVERRIDABLE_KINDS" {
    got="$(_kinds "$DECIDE")"
    for k in $PLANNING_KINDS; do
        [ "$(printf '%s\n' "$got" | grep -cx -- "$k")" -eq 0 ]
    done
}

@test "the pin fails when a kind is appended to OVERRIDABLE_KINDS" {
    # Prove the pin mechanism itself catches growth: a copy of the script with
    # an extra kind on the allowlist must not match the expected set. This is
    # the negative path — the same comparison that guards the real script.
    cp "$DECIDE" "$TMP/mutated.sh"
    sed -i 's/^OVERRIDABLE_KINDS="implementer explore-researcher refine-lens qa-sweep"$/OVERRIDABLE_KINDS="implementer explore-researcher refine-lens qa-sweep spec-design"/' "$TMP/mutated.sh"
    [ "$(_kinds "$TMP/mutated.sh")" != "$EXPECTED_ALLOWLIST" ]
}

@test "the pin fails when a kind is removed from OVERRIDABLE_KINDS" {
    cp "$DECIDE" "$TMP/mutated.sh"
    sed -i 's/^OVERRIDABLE_KINDS="implementer explore-researcher refine-lens qa-sweep"$/OVERRIDABLE_KINDS="implementer explore-researcher qa-sweep"/' "$TMP/mutated.sh"
    [ "$(_kinds "$TMP/mutated.sh")" != "$EXPECTED_ALLOWLIST" ]
}

# ── ledger vocabulary: the planning kinds are named ───────────────────────────

@test "the planning kinds are in the ledger vocabulary" {
    allowed="$(sed -n 's/^ALLOWED_KINDS="\([^"]*\)".*/\1/p' "$LEDGER" | head -n 1)"
    for k in $PLANNING_KINDS; do
        _found=0
        for w in $allowed; do
            [ "$w" = "$k" ] && _found=1
        done
        [ "$_found" -eq 1 ]
    done
}

@test "routing-ledger.sh accepts a record for each planning kind" {
    LEDGER_FILE="$TMP/ledger.jsonl"
    i=1
    for k in $PLANNING_KINDS; do
        run bash "$LEDGER" --ledger "$LEDGER_FILE" --append "$(rec "d$i" "$k")"
        [ "$status" -eq 0 ]
        i=$((i + 1))
    done
    run grep -c . "$LEDGER_FILE"
    [ "$output" = "4" ]
    run bash "$LEDGER" --ledger "$LEDGER_FILE" --validate
    [ "$status" -eq 0 ]
}

# ── route-decide.sh: the planning kinds fall through to the baseline ──────────

@test "planning kinds fall through to the baseline despite strong cheap-model evidence" {
    # The local profile fits the 120k/deep cell and has a strong record for the
    # named kind: only the allowlist can keep the baseline. For an overridable
    # kind the same evidence yields the local model (see
    # tests/routing-decision.bats), so this pins the rejection, not the parity.
    for k in $PLANNING_KINDS; do
        jq -n --argjson a "$(stats_row "$k" qwen3-32b-laptop 120k deep 50 0.95 0.02 0.02 0.1 0.0)" \
              --argjson b "$(stats_row "$k" claude-sonnet-cloud 120k deep 50 0.60 0.20 0.20 1.0 0.0)" \
              '[$a,$b]' > "$TMP/s.json"
        run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" \
            --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" --kind "$k"
        [ "$status" -eq 0 ]
        [ "$output" = "claude-sonnet-4-6" ]
    done
}

@test "an overridable kind still routes on the same evidence (control)" {
    # Control for the test above: with identical rows, an allowlisted kind MUST
    # take the cheaper local model, so the planning-kind tests cannot pass by
    # accident of an always-baseline script.
    jq -n --argjson a "$(stats_row refine-lens qwen3-32b-laptop 120k deep 50 0.95 0.02 0.02 0.1 0.0)" \
          --argjson b "$(stats_row refine-lens claude-sonnet-cloud 120k deep 50 0.60 0.20 0.20 1.0 0.0)" \
          '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" \
        --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" --kind refine-lens
    [ "$status" -eq 0 ]
    [ "$output" = "qwen3:32b" ]
}

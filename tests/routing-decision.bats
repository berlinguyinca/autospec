#!/usr/bin/env bats
# tests/routing-decision.bats — TDD for scripts/routing-cost.sh (scoring) and
# scripts/route-decide.sh (the decision).
#
# The load-bearing property is PARITY: with an empty or thin ledger,
# route-decide.sh must print exactly what select-model-profile.sh prints today. A
# router that silently changes routing on a host with no telemetry is worse than
# the status quo, so several tests below exist only to pin "no data = no change".

COST="${BATS_TEST_DIRNAME}/../scripts/routing-cost.sh"
DECIDE="${BATS_TEST_DIRNAME}/../scripts/route-decide.sh"
SELECTOR="${BATS_TEST_DIRNAME}/../skills/autospec-run/scripts/select-model-profile.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/routing-decision-XXXXXX")"
    EMPTY="$TMP/empty.json"
    printf '[]' > "$EMPTY"

    # Two cloud profiles spanning both cells, plus a local profile that fits the
    # top cell so the high-stakes exploration guard can be tested in isolation.
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

# stats_row <profile> <ctx> <reasoning> <n> <first_pass> <fail> <esc> <retries> <cache>
stats_row() {
    printf '{"dispatch_kind":"implementer","profile":"%s","cell_ctx":"%s","cell_reasoning":"%s","dispatches":%s,"first_pass_rate":%s,"failure_rate":%s,"escalation_rate":%s,"mean_retries":%s,"cache_hit_ratio":%s}' \
        "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9"
}

# ── routing-cost.sh ───────────────────────────────────────────────────────────

@test "routing-cost.sh and route-decide.sh are executable" {
    run test -x "$COST"
    [ "$status" -eq 0 ]
    run test -x "$DECIDE"
    [ "$status" -eq 0 ]
}

@test "routing-cost.sh requires the cell coordinates" {
    run bash "$COST" --kind implementer
    [ "$status" -eq 1 ]
}

@test "an empty ledger makes every candidate ineligible" {
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates "claude-haiku-cloud,claude-sonnet-cloud" --stats-file "$EMPTY"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq '[.[]|select(.eligible)]|length')" -eq 0 ]
    [[ "$output" == *"insufficient samples"* ]]
}

@test "a profile with no cost keys is ineligible and says why" {
    printf 'nocost-profile:\n  model: x\n  ctx: 64k\n  reasoning: medium\n' > "$PROF"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates "nocost-profile" --stats-file "$EMPTY"
    [ "$(printf '%s' "$output" | jq -r '.[0].unit')" = "null" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].eligible')" = "false" ]
    [[ "$output" == *"no cost keys"* ]]
}

@test "a cheap model that fails a lot costs MORE than a cheap reliable one" {
    # The central claim, stated against the tier a failing local model would
    # actually displace: qwen's unit cost is 0.2 against haiku's 6.0 — 30x
    # cheaper per token — yet retries, escalation and a broken prompt cache
    # invert the ranking. Compared against the DEAREST candidate the claim would
    # be trivially false, because that candidate absorbs the escalation term at
    # its own price; haiku is the honest comparison.
    jq -n --argjson a "$(stats_row qwen3-32b-laptop 120k deep 40 0.40 0.60 0.60 2.0 0.0)" \
          --argjson b "$(stats_row claude-haiku-cloud 120k deep 40 0.85 0.10 0.05 0.2 0.8)" \
          --argjson c "$(stats_row claude-sonnet-cloud 120k deep 40 0.92 0.04 0.02 0.15 0.8)" \
          '[$a,$b,$c]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" --kind implementer \
        --ctx 120k --reasoning deep --candidates "qwen3-32b-laptop,claude-haiku-cloud,claude-sonnet-cloud" --stats-file "$TMP/s.json"
    # Capture before any further `run`: bats resets $output on every run call.
    scored="$output"
    cheap_local="$(printf '%s' "$scored" | jq -r '.[]|select(.profile=="qwen3-32b-laptop")|.effective_cost')"
    cheap_cloud="$(printf '%s' "$scored" | jq -r '.[]|select(.profile=="claude-haiku-cloud")|.effective_cost')"
    run env A="$cheap_local" B="$cheap_cloud" python3 -c \
        "import os;a=float(os.environ['A']);b=float(os.environ['B']);assert a>b,(a,b)"
    [ "$status" -eq 0 ]
    # And the cheapest-per-token profile must not be the winner.
    [ "$(printf '%s' "$scored" | jq -r 'map(select(.eligible))|first|.profile')" = "claude-haiku-cloud" ]
}

@test "a failing profile is gated out by the first-pass floor" {
    jq -n --argjson a "$(stats_row qwen3-32b-laptop 120k deep 40 0.40 0.60 0.60 2.0 0.0)" '[$a]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" --kind implementer \
        --ctx 120k --reasoning deep --candidates "qwen3-32b-laptop" --stats-file "$TMP/s.json"
    [ "$(printf '%s' "$output" | jq -r '.[0].eligible')" = "false" ]
    [[ "$output" == *"below floor"* ]]
}

@test "a zero-cache profile carries a larger cache penalty" {
    jq -n --argjson a "$(stats_row claude-haiku-cloud 64k medium 40 0.9 0.05 0.02 0.1 0.0)" '[$a]' > "$TMP/none.json"
    jq -n --argjson a "$(stats_row claude-haiku-cloud 64k medium 40 0.9 0.05 0.02 0.1 1.0)" '[$a]' > "$TMP/full.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates "claude-haiku-cloud" --stats-file "$TMP/none.json"
    nc="$(printf '%s' "$output" | jq -r '.[0].cache_penalty')"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates "claude-haiku-cloud" --stats-file "$TMP/full.json"
    fc="$(printf '%s' "$output" | jq -r '.[0].cache_penalty')"
    run env A="$nc" B="$fc" python3 -c "import os;assert float(os.environ['A'])>float(os.environ['B'])"
    [ "$status" -eq 0 ]
}

@test "unproven profiles shrink toward pessimistic priors, never cheap ones" {
    # With no data, mean_retries must sit near the pessimistic prior (1.0), not 0.
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates "claude-haiku-cloud" --stats-file "$EMPTY"
    r="$(printf '%s' "$output" | jq -r '.[0].mean_retries')"
    run env R="$r" python3 -c "import os;assert float(os.environ['R'])>0.9"
    [ "$status" -eq 0 ]
}

# ── route-decide.sh: parity ───────────────────────────────────────────────────

@test "empty ledger: decision equals the baseline selector for every cell" {
    for lbl in "auto-implement,reasoning:shallow,ctx:32k" \
               "auto-implement,reasoning:medium,ctx:64k" \
               "auto-implement,reasoning:deep,ctx:120k" \
               "auto-implement,area:none"; do
        base="$(AUTOSPEC_MODEL_PROFILES="$PROF" bash "$SELECTOR" --labels "$lbl" --print-model 2>/dev/null || printf 'RC3')"
        run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" --labels "$lbl" --stats-file "$EMPTY"
        got="${output:-RC3}"
        [ "$got" = "$base" ]
    done
}

@test "unclassified labels fall back to the baseline" {
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" --labels "auto-implement" --stats-file "$EMPTY"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "a winner with no model: key falls back to the baseline" {
    cat > "$PROF" <<'EOF'
claude-sonnet-cloud:
  model: claude-sonnet-4-6
  ctx: 120k
  reasoning: deep
  cost_in: 3.0
  cost_out: 15.0
cheap-but-undispatchable:
  ctx: 120k
  reasoning: deep
  cost_in: 0.1
  cost_out: 0.1
EOF
    jq -n --argjson a "$(stats_row cheap-but-undispatchable 120k deep 50 0.95 0.02 0.01 0.1 0.8)" \
          --argjson b "$(stats_row claude-sonnet-cloud 120k deep 50 0.90 0.05 0.03 0.2 0.8)" \
          '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

# ── route-decide.sh: override, only when earned ───────────────────────────────

@test "a local profile that earned its record wins the cell" {
    jq -n --argjson a "$(stats_row qwen3-32b-laptop 120k deep 50 0.90 0.05 0.05 0.2 0.0)" \
          --argjson b "$(stats_row claude-sonnet-cloud 120k deep 50 0.92 0.04 0.02 0.15 0.8)" \
          '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json"
    [ "$status" -eq 0 ]
    [ "$output" = "qwen3:32b" ]
}

@test "the reviewer is never re-routed on the same evidence" {
    # Invariant: reviewer tier >= implementer tier. A 32B model reviewing
    # 32B-written code degrades quality invisibly and the ledger would record it
    # as a success.
    jq -n --argjson a "$(stats_row qwen3-32b-laptop 120k deep 50 0.90 0.05 0.05 0.2 0.0)" \
          --argjson b "$(stats_row claude-sonnet-cloud 120k deep 50 0.92 0.04 0.02 0.15 0.8)" \
          '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" --kind lgtm-reviewer
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "safety-gate dispatch kinds are never re-routed" {
    jq -n --argjson a "$(stats_row qwen3-32b-laptop 120k deep 50 0.90 0.05 0.05 0.2 0.0)" \
          --argjson b "$(stats_row claude-sonnet-cloud 120k deep 50 0.92 0.04 0.02 0.15 0.8)" \
          '[$a,$b]' > "$TMP/s.json"
    for k in secaudit-pass spec-decompose verify-voter; do
        run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" --kind "$k"
        [ "$output" = "claude-sonnet-4-6" ]
    done
}

@test "policy=off keeps the baseline despite strong evidence" {
    jq -n --argjson a "$(stats_row qwen3-32b-laptop 120k deep 50 0.90 0.05 0.05 0.2 0.0)" \
          --argjson b "$(stats_row claude-sonnet-cloud 120k deep 50 0.92 0.04 0.02 0.15 0.8)" \
          '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_ROUTING_POLICY=off AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
        --profiles-file "$PROF" --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json"
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "auto policy declines an override that is not strictly cheaper" {
    # Eligible but dearer than the baseline: standing aside is the correct move.
    jq -n --argjson a "$(stats_row qwen3-32b-laptop 120k deep 50 0.95 0.01 0.01 0.05 0.9)" \
          --argjson b "$(stats_row claude-sonnet-cloud 120k deep 50 0.95 0.01 0.01 0.05 0.9)" \
          '[$a,$b]' > "$TMP/s.json"
    # Make the local profile the expensive one.
    python3 - "$PROF" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
path.write_text(path.read_text().replace("  cost_minute: 0.02", "  cost_minute: 99.0"))
PY
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json"
    [ "$output" = "claude-sonnet-4-6" ]
}

# ── route-decide.sh: cold start (R8) ─────────────────────────────────────────

@test "cold-start exploration is off by default" {
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" --labels "auto-implement,reasoning:shallow,ctx:32k" --stats-file "$EMPTY"
    [ "$output" = "claude-haiku-4-5" ]
}

@test "exploration probes an unproven profile on the lowest-stakes cell" {
    run env AUTOSPEC_ROUTING_EXPLORE_PCT=100 AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
        --profiles-file "$PROF" --labels "auto-implement,reasoning:shallow,ctx:32k" --stats-file "$EMPTY"
    [ "$status" -eq 0 ]
    [ "$output" = "qwen3:32b" ]
}

@test "exploration never fires on a high-stakes cell even at 100 percent" {
    # The local profile FITS 120k/deep here, so only the cell guard can stop it.
    run env AUTOSPEC_ROUTING_EXPLORE_PCT=100 AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
        --profiles-file "$PROF" --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$EMPTY"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "the exploration draw is deterministic for the same labels" {
    first=""
    for i in 1 2 3; do
        run env AUTOSPEC_ROUTING_EXPLORE_PCT=50 AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
            --profiles-file "$PROF" --labels "auto-implement,reasoning:shallow,ctx:32k" --stats-file "$EMPTY"
        if [ -z "$first" ]; then first="$output"; fi
        [ "$output" = "$first" ]
    done
}

@test "a malformed explore percent is treated as off" {
    run env AUTOSPEC_ROUTING_EXPLORE_PCT=abc AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
        --profiles-file "$PROF" --labels "auto-implement,reasoning:shallow,ctx:32k" --stats-file "$EMPTY"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-haiku-4-5" ]
}

# stats_row_kind <kind> <profile> <ctx> <reasoning> <n> <first_pass> <fail> <esc> <retries> <cache>
# Same shape as stats_row but for a dispatch_kind other than implementer, so
# evidence for one kind can be shown NOT to leak into another.
stats_row_kind() {
    printf '{"dispatch_kind":"%s","profile":"%s","cell_ctx":"%s","cell_reasoning":"%s","dispatches":%s,"first_pass_rate":%s,"failure_rate":%s,"escalation_rate":%s,"mean_retries":%s,"cache_hit_ratio":%s}' \
        "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "${10}"
}

# ── the overridable set is an allowlist ───────────────────────────────────────

@test "high-fan-out read-and-report kinds are re-routed on their own evidence" {
    # explore-researcher / refine-lens / qa-sweep produce findings a later gate
    # re-checks, so a wrong answer is caught downstream rather than merged.
    for k in explore-researcher refine-lens qa-sweep; do
        jq -n --argjson a "$(stats_row_kind "$k" qwen3-32b-laptop 120k deep 50 0.90 0.05 0.05 0.2 0.0)" \
              --argjson b "$(stats_row_kind "$k" claude-sonnet-cloud 120k deep 50 0.60 0.20 0.20 1.0 0.0)" \
              '[$a,$b]' > "$TMP/s.json"
        run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" \
            --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" --kind "$k"
        [ "$status" -eq 0 ]
        [ "$output" = "qwen3:32b" ]
    done
}

@test "a dispatch kind nobody has allowlisted falls through to the baseline" {
    # The guard is an ALLOWLIST: a kind added to the ledger vocabulary later must
    # be baseline-only until someone deliberately opens it. A blocklist would
    # open every future kind by default and silently delete the invariant.
    jq -n --argjson a "$(stats_row_kind bogus-future-kind qwen3-32b-laptop 120k deep 50 0.90 0.05 0.05 0.2 0.0)" \
          --argjson b "$(stats_row_kind bogus-future-kind claude-sonnet-cloud 120k deep 50 0.60 0.20 0.20 1.0 0.0)" \
          '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" \
        --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" --kind bogus-future-kind
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "opening explore-researcher does not open the lgtm reviewer with it" {
    # Both kinds now appear in one ledger; the reviewer must still be baseline
    # even when the cheaper profile has a strong record on the explore rows.
    jq -n --argjson a "$(stats_row_kind explore-researcher qwen3-32b-laptop 120k deep 50 0.95 0.02 0.02 0.1 0.0)" \
          --argjson b "$(stats_row_kind lgtm-reviewer qwen3-32b-laptop 120k deep 50 0.95 0.02 0.02 0.1 0.0)" \
          '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" \
        --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" --kind lgtm-reviewer
    [ "$output" = "claude-sonnet-4-6" ]
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" \
        --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" --kind explore-researcher
    [ "$output" = "qwen3:32b" ]
}

@test "evidence for one overridable kind does not leak into another" {
    # Rows are keyed by (dispatch_kind, profile, cell). qa-sweep having no rows
    # must mean qa-sweep gets the baseline, not that it inherits explore's record.
    jq -n --argjson a "$(stats_row_kind explore-researcher qwen3-32b-laptop 120k deep 50 0.95 0.02 0.02 0.1 0.0)" \
          '[$a]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" \
        --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" --kind qa-sweep
    [ "$output" = "claude-sonnet-4-6" ]
}

#!/usr/bin/env bats
# tests/routing-cost-latency.bats — TDD for R6 (issue #3349):
#
#   Triviality floor. ctx:32k + reasoning:shallow is the lowest-stakes cell; a
#   local profile is eligible there only when the ledger shows it strictly
#   faster than the baseline on that same cell. No data -> no win -> floor
#   holds, so a fresh host routes the trivial cell to the baseline exactly as
#   select-model-profile.sh does.
#
#   Wall clock in effective cost. effective_cost counts cost_minute x measured
#   mean minutes, so a local model that is cheap per minute but ten times as
#   slow cannot masquerade as cheap. Cloud profiles (no per-minute rate) get
#   no wall-clock term; with no ledger data the term is 0 ("no data, no
#   change").

COST="${BATS_TEST_DIRNAME}/../scripts/routing-cost.sh"
DECIDE="${BATS_TEST_DIRNAME}/../scripts/route-decide.sh"
SELECTOR="${BATS_TEST_DIRNAME}/../skills/autospec-run/scripts/select-model-profile.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/routing-latency-XXXXXX")"
    PROF="$TMP/profiles.yml"
    cat > "$PROF" <<'EOF'
claude-haiku-cloud:
  model: claude-haiku-4-5
  ctx: 120k
  reasoning: deep
  cost_in: 0.1
  cost_out: 0.5
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
  cost_minute: 0.04
EOF
    EMPTY="$TMP/empty.json"
    printf '[]' > "$EMPTY"
}

teardown() { rm -rf "$TMP"; }

# row <profile> <ctx> <reasoning> <n> <fp> <fail> <esc> <retries> <cache> <wall_ms>
row() {
    printf '{"dispatch_kind":"implementer","profile":"%s","cell_ctx":"%s","cell_reasoning":"%s","dispatches":%s,"first_pass_rate":%s,"failure_rate":%s,"escalation_rate":%s,"mean_retries":%s,"cache_hit_ratio":%s,"mean_wall_clock_ms":%s}' \
        "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "${10}"
}

_cost() {
    # $1 ctx, $2 reasoning, $3 stats file, $4.. optional baseline profile
    local ctx="$1" reasoning="$2" stats="$3" baseline="${4:-}"
    local args=()
    [ -n "$baseline" ] && args+=("--baseline" "$baseline")
    env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" \
        --kind implementer --ctx "$ctx" --reasoning "$reasoning" \
        --candidates "qwen3-32b-laptop,claude-haiku-cloud" \
        --stats-file "$stats" "${args[@]+"${args[@]}"}"
}

_qwen() { printf '%s' "$1" | jq -r '.[]|select(.profile=="qwen3-32b-laptop")'; }
_haiku() { printf '%s' "$1" | jq -r '.[]|select(.profile=="claude-haiku-cloud")'; }

# ── triviality floor ──────────────────────────────────────────────────────────

@test "floor: a local profile with no measured wall clock is ineligible on 32k/shallow" {
    # Local has a healthy quality record but NO wall-clock field at all; the
    # baseline has data. No win is provable, so the floor holds.
    printf '{"dispatch_kind":"implementer","profile":"qwen3-32b-laptop","cell_ctx":"32k","cell_reasoning":"shallow","dispatches":50,"first_pass_rate":0.9,"failure_rate":0.05,"escalation_rate":0.05,"mean_retries":0.2,"cache_hit_ratio":0.8}' > "$TMP/q.json"
    jq -n --argjson a "$(cat "$TMP/q.json")" \
          --argjson b "$(row claude-haiku-cloud 32k shallow 50 0.92 0.04 0.02 0.1 0.8 60000)" '[$a,$b]' > "$TMP/s.json"
    run _cost 32k shallow "$TMP/s.json" claude-haiku-cloud
    [ "$status" -eq 0 ]
    q="$(_qwen "$output")"
    [ "$(printf '%s' "$q" | jq -r .is_local)" = "true" ]
    [ "$(printf '%s' "$q" | jq -r .trivial_floor)" = "true" ]
    [ "$(printf '%s' "$q" | jq -r .latency_win)" = "false" ]
    [ "$(printf '%s' "$q" | jq -r .eligible)" = "false" ]
    [[ "$(printf '%s' "$q" | jq -r .reason)" == *"triviality floor"* ]]
    # the baseline is untouched by the floor
    [ "$(_haiku "$output" | jq -r .eligible)" = "true" ]
}

@test "floor: a SLOWER local profile is ineligible even with both sides measured" {
    jq -n --argjson a "$(row qwen3-32b-laptop 32k shallow 50 0.90 0.05 0.05 0.2 0.8 90000)" \
          --argjson b "$(row claude-haiku-cloud 32k shallow 50 0.90 0.05 0.05 0.2 0.8 60000)" '[$a,$b]' > "$TMP/s.json"
    run _cost 32k shallow "$TMP/s.json" claude-haiku-cloud
    q="$(_qwen "$output")"
    [ "$(printf '%s' "$q" | jq -r .eligible)" = "false" ]
    [ "$(printf '%s' "$q" | jq -r .latency_win)" = "false" ]
    [[ "$(printf '%s' "$q" | jq -r .reason)" == *"triviality floor"* ]]
}

@test "floor: a measured latency win lifts the floor and the profile becomes eligible" {
    # Local: 2 min mean. Baseline: 15 min mean. Both measured -> win -> eligible.
    jq -n --argjson a "$(row qwen3-32b-laptop 32k shallow 50 0.90 0.05 0.05 0.2 0.8 120000)" \
          --argjson b "$(row claude-haiku-cloud 32k shallow 50 0.90 0.05 0.05 0.2 0.8 900000)" '[$a,$b]' > "$TMP/s.json"
    run _cost 32k shallow "$TMP/s.json" claude-haiku-cloud
    q="$(_qwen "$output")"
    [ "$(printf '%s' "$q" | jq -r .latency_win)" = "true" ]
    [ "$(printf '%s' "$q" | jq -r .eligible)" = "true" ]
    [ "$(printf '%s' "$q" | jq -r .reason)" = "" ]
}

@test "floor: no baseline row means no provable win, so the floor holds" {
    # Local is fast, but the baseline has NO stats row at all -> cannot
    # prove a win -> floor holds (no data means no change).
    jq -n --argjson a "$(row qwen3-32b-laptop 32k shallow 50 0.90 0.05 0.05 0.2 0.8 30000)" '[$a]' > "$TMP/s.json"
    run _cost 32k shallow "$TMP/s.json" claude-haiku-cloud
    [ "$(_qwen "$output" | jq -r .eligible)" = "false" ]
    [[ "$(_qwen "$output" | jq -r .reason)" == *"triviality floor"* ]]
}

@test "floor: without --baseline the floor holds even against a measured win" {
    # Callers that do not know the baseline (e.g. standalone scoring) cannot
    # claim a latency win; the floor is conservative by default.
    jq -n --argjson a "$(row qwen3-32b-laptop 32k shallow 50 0.90 0.05 0.05 0.2 0.8 120000)" \
          --argjson b "$(row claude-haiku-cloud 32k shallow 50 0.90 0.05 0.05 0.2 0.8 900000)" '[$a,$b]' > "$TMP/s.json"
    run _cost 32k shallow "$TMP/s.json"
    [ "$(_qwen "$output" | jq -r .eligible)" = "false" ]
}

@test "floor: it applies to every dispatch kind, not just implementers" {
    jq -n --argjson a "$(row qwen3-32b-laptop 32k shallow 50 0.90 0.05 0.05 0.2 0.8 90000)" \
          --argjson b "$(row claude-haiku-cloud 32k shallow 50 0.90 0.05 0.05 0.2 0.8 60000)" '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" \
        --kind lgtm-reviewer --ctx 32k --reasoning shallow \
        --candidates "qwen3-32b-laptop,claude-haiku-cloud" \
        --stats-file "$TMP/s.json" --baseline claude-haiku-cloud
    [ "$(printf '%s' "$output" | jq -r '.[]|select(.profile=="qwen3-32b-laptop")|.eligible')" = "false" ]
}

@test "floor: off the 32k/shallow cell the local profile is eligible without a win" {
    jq -n --argjson a "$(row qwen3-32b-laptop 64k medium 50 0.90 0.05 0.05 0.2 0.8 0)" \
          --argjson b "$(row claude-haiku-cloud 64k medium 50 0.90 0.05 0.05 0.2 0.8 60000)" '[$a,$b]' > "$TMP/s.json"
    run _cost 64k medium "$TMP/s.json" claude-haiku-cloud
    q="$(_qwen "$output")"
    [ "$(printf '%s' "$q" | jq -r .trivial_floor)" = "false" ]
    [ "$(printf '%s' "$q" | jq -r .eligible)" = "true" ]
}

# ── wall clock in effective cost ──────────────────────────────────────────────

@test "wall clock: the same local record is cheaper at 1 min and dearer than the baseline at 10 min" {
    # Identical quality rows, different measured latency. Cloud haiku unit is
    # 0.6; local unit is 0.04*10 = 0.4, so the wall-clock term (0.04/min) must
    # decide the ordering.
    local fast slow h
    jq -n --argjson a "$(row qwen3-32b-laptop 64k medium 50 0.90 0.05 0.05 0.2 0.8 60000)" \
          --argjson b "$(row claude-haiku-cloud 64k medium 50 0.90 0.05 0.05 0.2 0.8 60000)" '[$a,$b]' > "$TMP/s.json"
    fast="$(_cost 64k medium "$TMP/s.json" claude-haiku-cloud)"
    jq -n --argjson a "$(row qwen3-32b-laptop 64k medium 50 0.90 0.05 0.05 0.2 0.8 600000)" \
          --argjson b "$(row claude-haiku-cloud 64k medium 50 0.90 0.05 0.05 0.2 0.8 60000)" '[$a,$b]' > "$TMP/s.json"
    slow="$(_cost 64k medium "$TMP/s.json" claude-haiku-cloud)"
    h="$(_haiku "$fast" | jq -r .effective_cost)"
    run env R="$(_qwen "$fast" | jq -r .effective_cost)" H="$h" \
        python3 -c "import os; assert float(os.environ['R']) < float(os.environ['H'])"
    [ "$status" -eq 0 ]
    run env R="$(_qwen "$slow" | jq -r .effective_cost)" H="$h" \
        python3 -c "import os; assert float(os.environ['R']) > float(os.environ['H'])"
    [ "$status" -eq 0 ]
    # and the slow one must rank BELOW the fast one
    run env F="$(_qwen "$fast" | jq -r .effective_cost)" S="$(_qwen "$slow" | jq -r .effective_cost)" \
        python3 -c "import os; assert float(os.environ['F']) < float(os.environ['S'])"
    [ "$status" -eq 0 ]
}

@test "wall clock: cloud profiles carry no per-minute rate and get no wall-clock term" {
    jq -n --argjson a "$(row qwen3-32b-laptop 64k medium 50 0.90 0.05 0.05 0.2 0.8 0)" \
          --argjson b "$(row claude-haiku-cloud 64k medium 50 0.90 0.05 0.05 0.2 0.8 60000)" '[$a,$b]' > "$TMP/s.json"
    run _cost 64k medium "$TMP/s.json" claude-haiku-cloud
    [ "$status" -eq 0 ]
    # haiku effective cost must equal its no-wall-clock value exactly:
    # 0.6 * (1 + (0.2*50+5)/55) * 1.1 + ((2.5+2.5)/55 + (2.5+2.5)/55) * 0.6
    run env H="$(_haiku "$output" | jq -r .effective_cost)" python3 -c "
import os
h = float(os.environ['H'])
assert abs(h - (0.6 * (1 + 15/55) * 1.1 + (5/55 + 5/55) * 0.6)) < 1e-9, h"
    [ "$status" -eq 0 ]
}

@test "wall clock: an empty ledger leaves effective cost byte-identical to before R6" {
    # "No data means no change": with zero rows the new term is 0, so the cost
    # equals the pure token formula.
    run _cost 64k medium "$EMPTY"
    [ "$status" -eq 0 ]
    run env Q="$(_qwen "$output" | jq -r .effective_cost)" H="$(_haiku "$output" | jq -r .effective_cost)" python3 -c "
import os
q = float(os.environ['Q']); h = float(os.environ['H'])
# n=0, alpha=5: priors dominate fully (retries 1.0, esc 0.5, fail 0.5,
# cache penalty 1.5); strongest unit = 0.6.
assert abs(q - (0.4 * 2.0 * 1.5 + (0.5 + 0.5) * 0.6)) < 1e-9, q
assert abs(h - (0.6 * 2.0 * 1.5 + (0.5 + 0.5) * 0.6)) < 1e-9, h"
    [ "$status" -eq 0 ]
}

# ── route-decide.sh integration ───────────────────────────────────────────────

@test "route-decide: a measured latency win on 32k/shallow overrides the baseline" {
    jq -n --argjson a "$(row qwen3-32b-laptop 32k shallow 50 0.90 0.05 0.05 0.2 0.8 120000)" \
          --argjson b "$(row claude-haiku-cloud 32k shallow 50 0.90 0.05 0.05 0.2 0.8 900000)" '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
        --profiles-file "$PROF" --stats-file "$TMP/s.json" \
        --labels "auto-implement,ctx:32k,reasoning:shallow"
    [ "$status" -eq 0 ]
    [ "$output" = "qwen3:32b" ]
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
        --profiles-file "$PROF" --stats-file "$TMP/s.json" \
        --labels "auto-implement,ctx:32k,reasoning:shallow" --print-profile
    [ "$output" = "qwen3-32b-laptop" ]
}

@test "route-decide: without a latency win the floor keeps 32k/shallow on the baseline" {
    jq -n --argjson a "$(row qwen3-32b-laptop 32k shallow 50 0.90 0.05 0.05 0.2 0.8 90000)" \
          --argjson b "$(row claude-haiku-cloud 32k shallow 50 0.90 0.05 0.05 0.2 0.8 60000)" '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
        --profiles-file "$PROF" --stats-file "$TMP/s.json" \
        --labels "auto-implement,ctx:32k,reasoning:shallow"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-haiku-4-5" ]
}

@test "route-decide: the floor does not block cold-start exploration" {
    # Empty ledger + exploration enabled: the floor keeps the local profile out
    # of the ELIGIBLE set, but it remains a legal probe target (that is how the
    # latency data that lifts the floor ever gets collected).
    run env AUTOSPEC_MODEL_PROFILES="$PROF" AUTOSPEC_ROUTING_EXPLORE_PCT=100 bash "$DECIDE" \
        --profiles-file "$PROF" --stats-file "$EMPTY" \
        --labels "auto-implement,ctx:32k,reasoning:shallow"
    [ "$status" -eq 0 ]
    [ "$output" = "qwen3:32b" ]
}

@test "route-decide: an empty ledger prints exactly the baseline selector's model for every cell" {
    for lbl in "auto-implement,reasoning:shallow,ctx:32k" \
               "auto-implement,reasoning:medium,ctx:64k" \
               "auto-implement,reasoning:deep,ctx:120k" \
               "auto-implement,area:none"; do
        base="$(AUTOSPEC_MODEL_PROFILES="$PROF" bash "$SELECTOR" --profiles-file "$PROF" --labels "$lbl" --print-model 2>/dev/null || printf 'RC3')"
        run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
            --profiles-file "$PROF" --labels "$lbl" --stats-file "$EMPTY"
        [ "$status" -eq 0 ]
        got="${output:-RC3}"
        [ "$got" = "$base" ]
    done
}

@test "route-decide: wall clock alone can flip an off-floor cell from local to baseline" {
    # 64k/medium (no floor). Local is token-cheap but 10x slower -> must lose.
    jq -n --argjson a "$(row qwen3-32b-laptop 64k medium 50 0.90 0.05 0.05 0.2 0.8 600000)" \
          --argjson b "$(row claude-haiku-cloud 64k medium 50 0.90 0.05 0.05 0.2 0.8 60000)" '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
        --profiles-file "$PROF" --stats-file "$TMP/s.json" \
        --labels "auto-implement,ctx:64k,reasoning:medium"
    [ "$status" -eq 0 ]
    [ "$output" = "claude-haiku-4-5" ]
    # and at 1 min the same quality wins -> local
    jq -n --argjson a "$(row qwen3-32b-laptop 64k medium 50 0.90 0.05 0.05 0.2 0.8 60000)" \
          --argjson b "$(row claude-haiku-cloud 64k medium 50 0.90 0.05 0.05 0.2 0.8 60000)" '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
        --profiles-file "$PROF" --stats-file "$TMP/s.json" \
        --labels "auto-implement,ctx:64k,reasoning:medium"
    [ "$output" = "qwen3:32b" ]
}

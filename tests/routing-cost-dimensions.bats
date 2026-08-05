#!/usr/bin/env bats
# tests/routing-cost-dimensions.bats — the two cost dimensions added after the
# original scorer landed: per-model prompt-cache minimums, and effort.
#
# Split out of tests/routing-decision.bats, which hit the 400-LOC module cap.
# That file owns PARITY (no data = no change) and the overridable-kind allowlist;
# this one owns the two dimensions that change how a FITTING profile is priced.

COST="${BATS_TEST_DIRNAME}/../scripts/routing-cost.sh"
DECIDE="${BATS_TEST_DIRNAME}/../scripts/route-decide.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/routing-dimensions-XXXXXX")"
    PROF="$TMP/profiles.yml"
    cat > "$PROF" <<'YAML'
claude-haiku-cloud:
  model: claude-haiku-4-5
  ctx: 64k
  reasoning: medium
  cost_in: 1.0
  cost_out: 5.0
  allowed: ctx:medium,reasoning:medium
YAML
}

teardown() { rm -rf "$TMP"; }

# stats_row <profile> <ctx> <reasoning> <n> <first_pass> <fail> <esc> <retries> <cache>
stats_row() {
    printf '{"dispatch_kind":"implementer","profile":"%s","cell_ctx":"%s","cell_reasoning":"%s","dispatches":%s,"first_pass_rate":%s,"failure_rate":%s,"escalation_rate":%s,"mean_retries":%s,"cache_hit_ratio":%s}' \
        "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9"
}

# ── cache_min_tokens: a prefix below the floor cannot cache ───────────────────

@test "a prefix below a profile's cache floor zeroes its measured cache credit" {
    # Prompt caches have a per-model MINIMUM — Haiku 4.5 needs 4096 tokens where
    # Opus 5 needs 512 — so the cheapest per-token profile is the easiest to fall
    # under, and a hit ratio measured under a larger prefix must not be credited.
    PROF2="$TMP/floors.yml"
    cat > "$PROF2" <<'YAML'
haiku-floor:
  model: claude-haiku-4-5
  ctx: 64k
  reasoning: medium
  cost_in: 1.0
  cost_out: 5.0
  cache_min_tokens: 4096
opus-floor:
  model: claude-opus-5
  ctx: 64k
  reasoning: medium
  cost_in: 5.0
  cost_out: 25.0
  cache_min_tokens: 512
YAML
    jq -n --argjson a "$(stats_row haiku-floor 64k medium 50 0.90 0.02 0.02 0.1 0.9)" \
          --argjson b "$(stats_row opus-floor 64k medium 50 0.90 0.02 0.02 0.1 0.9)" \
          '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF2" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates haiku-floor,opus-floor \
        --stats-file "$TMP/s.json" --prefix-tokens 2000 --json
    [ "$status" -eq 0 ]
    scored="$output"
    [ "$(printf '%s' "$scored" | jq -r '.[]|select(.profile=="haiku-floor")|.cache_hit_ratio')" = "0" ]
    [ "$(printf '%s' "$scored" | jq -r '.[]|select(.profile=="haiku-floor")|.cache_floor_unmet')" = "true" ]
    # The profile that clears its own floor keeps its measured credit.
    [ "$(printf '%s' "$scored" | jq -r '.[]|select(.profile=="opus-floor")|.cache_hit_ratio')" = "0.9" ]
    [ "$(printf '%s' "$scored" | jq -r '.[]|select(.profile=="opus-floor")|.cache_floor_unmet')" = "false" ]
}

@test "an unmet cache floor makes the cheap profile measurably dearer" {
    PROF2="$TMP/floors2.yml"
    cat > "$PROF2" <<'YAML'
haiku-floor:
  model: claude-haiku-4-5
  ctx: 64k
  reasoning: medium
  cost_in: 1.0
  cost_out: 5.0
  cache_min_tokens: 4096
YAML
    jq -n --argjson a "$(stats_row haiku-floor 64k medium 50 0.90 0.02 0.02 0.1 0.9)" \
          '[$a]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF2" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates haiku-floor \
        --stats-file "$TMP/s.json" --json
    cheap_when_cached="$(printf '%s' "$output" | jq -r '.[0].effective_cost')"
    run env AUTOSPEC_MODEL_PROFILES="$PROF2" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates haiku-floor \
        --stats-file "$TMP/s.json" --prefix-tokens 2000 --json
    dearer_when_not="$(printf '%s' "$output" | jq -r '.[0].effective_cost')"
    run jq -n --argjson a "$cheap_when_cached" --argjson b "$dearer_when_not" '$b > $a'
    [ "$output" = "true" ]
}

@test "an unknown prefix size leaves scoring exactly as it was" {
    # Fails open: a host that cannot report prefix size must score as before.
    jq -n --argjson a "$(stats_row claude-haiku-cloud 64k medium 50 0.90 0.02 0.02 0.1 0.9)" \
          '[$a]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates claude-haiku-cloud \
        --stats-file "$TMP/s.json" --json
    baseline_cost="$(printf '%s' "$output" | jq -r '.[0].effective_cost')"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates claude-haiku-cloud \
        --stats-file "$TMP/s.json" --prefix-tokens 0 --json
    [ "$(printf '%s' "$output" | jq -r '.[0].effective_cost')" = "$baseline_cost" ]
}

@test "a profile with no cache_min_tokens is never penalised by a prefix size" {
    # PROF's entries declare no floor, so nothing is knowable and nothing changes.
    jq -n --argjson a "$(stats_row claude-haiku-cloud 64k medium 50 0.90 0.02 0.02 0.1 0.9)" \
          '[$a]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates claude-haiku-cloud \
        --stats-file "$TMP/s.json" --prefix-tokens 10 --json
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.[0].cache_floor_unmet')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].cache_hit_ratio')" = "0.9" ]
}

# ── effort as a routable dimension ────────────────────────────────────────────

@test "--print-effort reports the baseline profile's effort tier" {
    PROF3="$TMP/effort.yml"
    cat > "$PROF3" <<'YAML'
claude-haiku-cloud:
  model: claude-haiku-4-5
  ctx: 64k
  reasoning: medium
  effort: low
  allowed: ctx:medium,reasoning:medium
YAML
    run env AUTOSPEC_MODEL_PROFILES="$PROF3" bash "$DECIDE" --profiles-file "$PROF3" \
        --labels "auto-implement,ctx:medium,reasoning:medium" --print-effort
    [ "$status" -eq 0 ]
    [ "$output" = "low" ]
}

@test "a profile stating no effort exits 3 so the caller keeps its own default" {
    # Guessing an effort tier is the same class of error as guessing a model id.
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" --profiles-file "$PROF" \
        --labels "auto-implement,ctx:medium,reasoning:medium" --print-effort
    [ "$status" -eq 3 ]
    [ -z "$output" ]
}

@test "effort follows the overridden winner, never the baseline it replaced" {
    # Pairing the baseline's effort with an overridden model would report a tier
    # that model was never measured at.
    PROF4="$TMP/effort-override.yml"
    cat > "$PROF4" <<'YAML'
claude-sonnet-cloud:
  model: claude-sonnet-5
  ctx: 120k
  reasoning: deep
  effort: high
  cost_in: 3.0
  cost_out: 15.0
qwen3-32b-laptop:
  model: qwen3:32b
  ctx: 120k
  reasoning: deep
  effort: medium
  cost_minute: 0.02
YAML
    jq -n --argjson a "$(stats_row qwen3-32b-laptop 120k deep 50 0.95 0.02 0.02 0.1 0.0)" \
          --argjson b "$(stats_row claude-sonnet-cloud 120k deep 50 0.60 0.20 0.20 1.0 0.0)" \
          '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF4" bash "$DECIDE" --profiles-file "$PROF4" \
        --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json"
    [ "$output" = "qwen3:32b" ]
    run env AUTOSPEC_MODEL_PROFILES="$PROF4" bash "$DECIDE" --profiles-file "$PROF4" \
        --labels "auto-implement,reasoning:deep,ctx:120k" --stats-file "$TMP/s.json" --print-effort
    [ "$status" -eq 0 ]
    [ "$output" = "medium" ]
}

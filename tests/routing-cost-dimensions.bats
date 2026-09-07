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

# ── §24–§28 multidimensional smoothed stats (routing-ledger.sh --group-by) ────
#
# One row per cell over any subset of the 13 §24 dimensions. A thin cell backs
# its RATES off to its parent (drop the rightmost dim) with $alpha pseudo-counts;
# an empty cell reports "unknown" rather than the prior. Medians, percentiles and
# token rates stay raw — smoothing a median invents a number nobody measured.

LGR="${BATS_TEST_DIRNAME}/../scripts/routing-ledger.sh"

# r <id> <kind> <model> <ctx> <outcome> [retries]
r() {
    printf '{"dispatch_id":"%s","ts":"2026-08-16T10:00:00Z","dispatch_kind":"%s","profile":"p","model":"%s","harness":"claude-code","issue":"#1","cell_ctx":"%s","cell_reasoning":"medium","input_tokens":1000,"output_tokens":200,"cached_tokens":200,"wall_clock_ms":60000,"retries":%s,"escalated":false,"outcome":"%s","reason":""}' \
        "$1" "$2" "$3" "$4" "${6:-0}" "$5"
}

# 6 implementer/64k (3 first-pass) + 3 qa-sweep/32k (2 first-pass). Parent rate is
# 5/9, so the thin cell's blend 0.597 differs from both its raw 0.667 and 0.556.
nine_ledger() {
    { r d1 implementer big 64k merged_clean
      r d2 implementer big 64k merged_clean
      r d3 implementer big 64k merged_clean
      r d4 implementer big 64k qa_failed 1
      r d5 implementer big 64k qa_failed 2
      r d6 implementer big 64k retried_ok 1
      r q1 qa-sweep big 32k merged_clean
      r q2 qa-sweep big 32k merged_clean
      r q3 qa-sweep big 32k qa_failed; } > "$1"
}

# gstats <ledger> <group-by> [extra flags...] — MIN_SAMPLES=5 so the 6-row cell
# clears the bar and the 3-row cell does not.
gstats() {
    local l="$1" gb="$2"; shift 2
    run env AUTOSPEC_ROUTING_MIN_SAMPLES=5 bash "$LGR" --ledger "$l" --stats --json \
        --group-by "$gb" "$@"
}

@test "--group-by emits one row per cell and every §26 metric as a column" {
    nine_ledger "$TMP/l.jsonl"
    run env AUTOSPEC_ROUTING_MIN_SAMPLES=5 bash "$LGR" --ledger "$TMP/l.jsonl" \
        --stats --group-by dispatch_kind
    [ "$status" -eq 0 ]
    header="$(printf '%s\n' "$output" | head -1)"
    for col in dispatches evidence first_pass_rate eventual_success_rate mean_retries \
               median_retries escalation_rate review_rejection_rate test_failure_rate \
               revert_rate median_completion_ms p95_completion_ms prompt_tok_s \
               decode_tok_s cache_hit_ratio cost_per_success; do
        printf '%s' "$header" | tr '\t' '\n' | grep -qx "$col"
    done
    [ "$(printf '%s\n' "$output" | tail -n +2 | wc -l | tr -d ' ')" = "2" ]
}

@test "a cell at min_samples reports its raw rate; a thinner one reports the blend" {
    nine_ledger "$TMP/l.jsonl"
    gstats "$TMP/l.jsonl" dispatch_kind
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.[0].evidence')" = "observed" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].first_pass_rate')" = "0.5" ]
    [ "$(printf '%s' "$output" | jq -r '.[1].evidence')" = "smoothed" ]
    [ "$(printf '%s' "$output" | jq -r '.[1].dispatches')" = "3" ]
    [ "$(printf '%s' "$output" | jq -r '.[1].first_pass_rate')" = "$(jq -n '(2 + 5*(5/9)) / (3 + 5)')" ]
    # A single-level group-by has no parent cell to name, only the root count.
    [ "$(printf '%s' "$output" | jq -r '.[1].parent')" = "null" ]
    [ "$(printf '%s' "$output" | jq -r '.[1].parent_dispatches')" = "9" ]
}

@test "alpha=0 leaves a smoothed cell on its raw rate but keeps the label honest" {
    nine_ledger "$TMP/l.jsonl"
    run env AUTOSPEC_ROUTING_ALPHA=0 AUTOSPEC_ROUTING_MIN_SAMPLES=5 bash "$LGR" \
        --ledger "$TMP/l.jsonl" --stats --json --group-by dispatch_kind
    [ "$(printf '%s' "$output" | jq -r '.[1].evidence')" = "smoothed" ]
    [ "$(printf '%s' "$output" | jq -r '.[1].first_pass_rate')" = "0.6666666666666666" ]
}

@test "raising min_samples reclassifies the same cell from observed to smoothed" {
    nine_ledger "$TMP/l.jsonl"
    run env AUTOSPEC_ROUTING_MIN_SAMPLES=7 bash "$LGR" --ledger "$TMP/l.jsonl" \
        --stats --json --group-by dispatch_kind
    [ "$(printf '%s' "$output" | jq -r '.[0].evidence')" = "smoothed" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].first_pass_rate')" = "$(jq -n '(3 + 5*(5/9)) / (6 + 5)')" ]
}

@test "a smoothed cell still reports raw medians, percentiles and cache ratio" {
    nine_ledger "$TMP/l.jsonl"
    gstats "$TMP/l.jsonl" dispatch_kind
    [ "$(printf '%s' "$output" | jq -r '.[1].median_completion_ms')" = "60000" ]
    [ "$(printf '%s' "$output" | jq -r '.[1].p95_completion_ms')" = "60000" ]
    [ "$(printf '%s' "$output" | jq -r '.[1].cache_hit_ratio')" = "0.2" ]
    [ "$(printf '%s' "$output" | jq -r '.[1].median_retries')" = "0" ]
}

@test "a two-level group-by names its parent cell and the parent's dispatch count" {
    nine_ledger "$TMP/l.jsonl"
    gstats "$TMP/l.jsonl" model,context_band
    [ "$(printf '%s' "$output" | jq -r 'length')" = "2" ]
    [ "$(printf '%s' "$output" | jq -c '.[0].parent')" = '{"model":"big"}' ]
    [ "$(printf '%s' "$output" | jq -r '.[0].parent_dispatches')" = "9" ]
    # 32k is thin, so it blends toward the 9-row model cell; 64k clears the bar.
    [ "$(printf '%s' "$output" | jq -r '.[0].context_band')" = "32k" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].evidence')" = "smoothed" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].first_pass_rate')" = "$(jq -n '(2 + 5*(5/9)) / (3 + 5)')" ]
    [ "$(printf '%s' "$output" | jq -r '.[1].evidence')" = "observed" ]
    [ "$(printf '%s' "$output" | jq -r '.[1].first_pass_rate')" = "0.5" ]
}

@test "an empty --cell reports unknown, not the parent prior" {
    nine_ledger "$TMP/l.jsonl"
    gstats "$TMP/l.jsonl" model,context_band --cell model=ghost,context_band=999k
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.[0].dispatches')" = "0" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].evidence')" = "unknown" ]
    for m in first_pass_rate eventual_success_rate escalation_rate median_completion_ms cost_per_success; do
        [ "$(printf '%s' "$output" | jq -r ".[0].$m")" = "unknown" ]
    done
    [ "$(printf '%s' "$output" | jq -r '.[0].parent')" = "null" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].parent_dispatches')" = "0" ]
}

@test "--cell narrows the report to exactly the addressed cell" {
    nine_ledger "$TMP/l.jsonl"
    gstats "$TMP/l.jsonl" model,context_band --cell model=big,context_band=32k
    [ "$(printf '%s' "$output" | jq -r 'length')" = "1" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].context_band')" = "32k" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].dispatches')" = "3" ]
}

@test "a --cell naming a dimension outside --group-by is a usage error" {
    nine_ledger "$TMP/l.jsonl"
    gstats "$TMP/l.jsonl" dispatch_kind --cell language=rust
    [ "$status" -eq 1 ]
    [[ "$output" == *"--cell dimension"* ]]
}

@test "a --cell that leaves a --group-by dimension unaddressed is a usage error" {
    nine_ledger "$TMP/l.jsonl"
    gstats "$TMP/l.jsonl" model,context_band --cell model=big
    [ "$status" -eq 1 ]
    [[ "$output" == *"every --group-by dimension"* ]]
}

@test "a malformed --cell entry without = is a usage error" {
    nine_ledger "$TMP/l.jsonl"
    gstats "$TMP/l.jsonl" model --cell small-32b
    [ "$status" -eq 1 ]
    [[ "$output" == *"expected k=v"* ]]
}

@test "an unknown or repeated --group-by dimension is a usage error" {
    nine_ledger "$TMP/l.jsonl"
    gstats "$TMP/l.jsonl" bogus
    [ "$status" -eq 1 ]
    [[ "$output" == *"unknown dimension(s): bogus"* ]]
    gstats "$TMP/l.jsonl" model,model
    [ "$status" -eq 1 ]
    [[ "$output" == *"duplicate dimension(s): model"* ]]
}

@test "--group-by needs --stats and --cell needs --group-by" {
    nine_ledger "$TMP/l.jsonl"
    run bash "$LGR" --ledger "$TMP/l.jsonl" --group-by model
    [ "$status" -eq 1 ]
    run bash "$LGR" --ledger "$TMP/l.jsonl" --stats --cell model=big
    [ "$status" -eq 1 ]
}

@test "metrics whose #3174 telemetry field is absent report unknown" {
    nine_ledger "$TMP/l.jsonl"
    gstats "$TMP/l.jsonl" dispatch_kind
    [ "$(printf '%s' "$output" | jq -r '.[0].review_rejection_rate')" = "unknown" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].test_failure_rate')" = "unknown" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].cost_per_success')" = "unknown" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].prompt_tok_s')" = "unknown" ]
}

@test "once the #3174 fields land, the same metrics report real rates" {
    { r a1 implementer big 64k merged_clean
      r a2 implementer big 64k merged_clean
      r a3 implementer big 64k merged_clean
      r a4 implementer big 64k merged_clean; } > "$TMP/x.jsonl"
    # Two of four reviews rejected, two of four test runs failed — over rows that
    # REPORT the field, so a row answering "approved" counts in the denominator.
    # Cost is recorded on only two rows: the unit cost divides priced successes,
    # not every success, or it would read half the true figure.
    jq -c '. + {review_outcome:"rejected",tests_outcome:"failed",cost_usd:1,prompt_tok_s:100}' \
        "$TMP/x.jsonl" | head -2 > "$TMP/y.jsonl"
    jq -c '. + {review_outcome:"approved",tests_outcome:"passed"}' "$TMP/x.jsonl" | tail -2 >> "$TMP/y.jsonl"
    run env AUTOSPEC_ROUTING_MIN_SAMPLES=4 bash "$LGR" --ledger "$TMP/y.jsonl" \
        --stats --json --group-by dispatch_kind
    [ "$(printf '%s' "$output" | jq -r '.[0].review_rejection_rate')" = "0.5" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].test_failure_rate')" = "0.5" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].cost_per_success')" = "1" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].prompt_tok_s')" = "100" ]
}

@test "context_band reads cell_ctx first and derives from context_used otherwise" {
    { r c1 implementer big 32k merged_clean | jq -c '.cell_ctx=null | .context_used=30000'
      r c2 implementer big 32k qa_failed | jq -c '.cell_ctx=null | .context_used=100000'
      r c3 implementer big 32k qa_failed | jq -c 'del(.cell_ctx) | del(.context_used)'; } > "$TMP/c.jsonl"
    run env AUTOSPEC_ROUTING_MIN_SAMPLES=2 bash "$LGR" --ledger "$TMP/c.jsonl" \
        --stats --group-by context_band
    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | tail -n +2 | cut -f1 | sort | tr '\n' ' ')" = "120k 32k unknown " ]
}

@test "plain --stats keeps its pre-#3175 shape for routing-cost.sh and the old tests" {
    nine_ledger "$TMP/l.jsonl"
    run bash "$LGR" --ledger "$TMP/l.jsonl" --stats --json
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.[0] | has("evidence")')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.[0] | has("parent")')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].dispatches')" = "6" ]
    run bash "$LGR" --ledger "$TMP/l.jsonl" --stats
    [[ "$(printf '%s\n' "$output" | head -1)" == *"first_pass=0.5"* ]]
}

# ── the shared smoothing contract: routing-cost.sh --jq-prelude ───────────────

@test "--jq-prelude publishes the alpha, min_samples and smooth definition" {
    run bash "$COST" --jq-prelude
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.alpha')" = "5" ]
    [ "$(printf '%s' "$output" | jq -r '.min_samples')" = "10" ]
    [[ "$(printf '%s' "$output" | jq -r '.jq')" == *"def smooth"* ]]
    run env AUTOSPEC_ROUTING_ALPHA=2 AUTOSPEC_ROUTING_MIN_SAMPLES=3 bash "$COST" --jq-prelude
    [ "$(printf '%s' "$output" | jq -r '.alpha')" = "2" ]
    [ "$(printf '%s' "$output" | jq -r '.min_samples')" = "3" ]
    # A non-numeric contract fails closed instead of silently defaulting.
    run env AUTOSPEC_ROUTING_ALPHA=abc bash "$COST" --jq-prelude
    [ "$status" -eq 1 ]
}

@test "§27: an advertised first-pass rate is a prior only where nothing was observed" {
    PROFA="$TMP/advertised.yml"
    cat > "$PROFA" <<'YAML'
claimed-model:
  model: some-claimed-4
  ctx: 64k
  reasoning: medium
  cost_in: 1.0
  cost_out: 5.0
  advertised_first_pass: 0.8
bogus-claim:
  model: some-claimed-9
  ctx: 64k
  reasoning: medium
  cost_in: 1.0
  cost_out: 5.0
  advertised_first_pass: 5
YAML
    printf '[]' > "$TMP/none.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROFA" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates claimed-model,bogus-claim \
        --stats-file "$TMP/none.json" --json
    [ "$(printf '%s' "$output" | jq -r '.[]|select(.profile=="claimed-model")|.first_pass_source')" = "advertised" ]
    [ "$(printf '%s' "$output" | jq -r '.[]|select(.profile=="claimed-model")|.first_pass_prior')" = "0.8" ]
    # An out-of-range claim is not evidence; it is treated as absent.
    [ "$(printf '%s' "$output" | jq -r '.[]|select(.profile=="bogus-claim")|.first_pass_source')" = "none" ]
    [ "$(printf '%s' "$output" | jq -r '.[]|select(.profile=="bogus-claim")|.first_pass_prior')" = "0.5" ]
    # Six real dispatches outrank the vendor's 0.8.
    jq -n --argjson a "$(stats_row claimed-model 64k medium 6 0.5 0.2 0.1 0.5 0)" '[$a]' > "$TMP/obs.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROFA" bash "$COST" --kind implementer \
        --ctx 64k --reasoning medium --candidates claimed-model \
        --stats-file "$TMP/obs.json" --json
    [ "$(printf '%s' "$output" | jq -r '.[0].first_pass_source')" = "observed" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].first_pass_prior')" = "0.5" ]
}

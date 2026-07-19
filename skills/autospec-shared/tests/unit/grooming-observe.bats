#!/usr/bin/env bats
# Tests for grooming-observe.sh — derives the groomed vs baseline clean-merge
# rate from autospec's telemetry JSONL, feeding grooming-govern.sh's tick.

setup() {
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/grooming-observe.sh"
  TMP="$(mktemp -d)"
  T="$TMP/telemetry.jsonl"
  # 2 template-groomed issues (1 clean, 1 escalated) + 2 baseline (non-template-
  # groom) issues (both clean). template_groomed:true marks records the
  # reworked script now partitions on (v1 partitioned on the generic
  # groomed/source fields instead); no outcome field, so is_resolved/is_clean
  # fall back to the legacy reverted/reopened/labels shape.
  cat > "$T" <<'EOF'
{"issue":"1","template_groomed":true,"groomed":true,"reverted":false,"reopened":false,"labels":[]}
{"issue":"2","template_groomed":true,"groomed":true,"reverted":false,"reopened":false,"labels":["escalate:human"]}
{"issue":"3","groomed":false,"reverted":false,"reopened":false,"labels":[]}
{"issue":"4","groomed":false,"reverted":false,"reopened":false,"labels":[]}
EOF
}

teardown() { rm -rf "$TMP"; }

@test "computes groomed_clean_merge_rate and samples from groomed issues" {
  run bash "$SCRIPT" --telemetry "$T" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.groomed_clean_merge_rate == 0.5' >/dev/null
  echo "$output" | jq -e '.samples == 2' >/dev/null
}

@test "computes baseline_clean_merge_rate from ungroomed issues" {
  run bash "$SCRIPT" --telemetry "$T" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.baseline_clean_merge_rate == 1' >/dev/null
}

@test "emits baseline_samples counting ungroomed records (widen-guard input)" {
  run bash "$SCRIPT" --telemetry "$T" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.baseline_samples == 2' >/dev/null
}

@test "zeroed metrics include baseline_samples:0" {
  : > "$TMP/empty.jsonl"
  run bash "$SCRIPT" --telemetry "$TMP/empty.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.baseline_samples == 0' >/dev/null
}

@test "empty/missing telemetry yields zeroed metrics, exit 0 (fail-safe)" {
  : > "$TMP/empty.jsonl"
  run bash "$SCRIPT" --telemetry "$TMP/empty.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.groomed_clean_merge_rate == 0 and .baseline_clean_merge_rate == 0 and .samples == 0' >/dev/null
}

@test "malformed telemetry lines are skipped, not fatal" {
  printf 'garbage not json\n' >> "$T"
  run bash "$SCRIPT" --telemetry "$T" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.samples == 2' >/dev/null
}

@test "nonexistent telemetry path yields zeroed metrics, exit 0" {
  run bash "$SCRIPT" --telemetry "$TMP/nope.jsonl" --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.samples == 0' >/dev/null
}

@test "samples counts only resolved template-groom records" {
  t="$BATS_TEST_TMPDIR/t.jsonl"
  {
    printf '{"issue":1,"template_groomed":true,"outcome":"clean"}\n'
    printf '{"issue":2,"template_groomed":true,"outcome":"rejected"}\n'
    printf '{"issue":3,"template_groomed":true,"outcome":null}\n'   # unresolved: excluded
    printf '{"issue":4,"template_groomed":false,"outcome":"clean"}\n' # baseline
  } > "$t"
  run bash "$SCRIPT" --telemetry "$t"
  [ "$status" -eq 0 ]
  [ "$(printf '%s' "$output" | jq -r .samples)" = "2" ]
  [ "$(printf '%s' "$output" | jq -r .groomed_clean_merge_rate)" = "0.5" ]
  [ "$(printf '%s' "$output" | jq -r .baseline_samples)" = "1" ]
  [ "$(printf '%s' "$output" | jq -r .baseline_clean_merge_rate)" = "1" ]
}

@test "back-compat: pre-enhancement record without outcome uses reverted/reopened" {
  t="$BATS_TEST_TMPDIR/t.jsonl"
  printf '{"issue":1,"template_groomed":true,"reverted":false,"reopened":false}\n' > "$t"
  # No outcome field → treated as resolved-clean via legacy fallback.
  run bash "$SCRIPT" --telemetry "$t"
  [ "$(printf '%s' "$output" | jq -r .samples)" = "1" ]
  [ "$(printf '%s' "$output" | jq -r .groomed_clean_merge_rate)" = "1" ]
}

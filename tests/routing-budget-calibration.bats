#!/usr/bin/env bats
# tests/routing-budget-calibration.bats — TDD for R9/R10/R11:
#   scripts/routing-budget-hint.sh    budget pressure -> paid-tier penalty
#   scripts/routing-cost.sh           wall-clock ceiling gate
#   scripts/calibrate-profile.sh      qualify a profile before trusting it

HINT="${BATS_TEST_DIRNAME}/../scripts/routing-budget-hint.sh"
COST="${BATS_TEST_DIRNAME}/../scripts/routing-cost.sh"
DECIDE="${BATS_TEST_DIRNAME}/../scripts/route-decide.sh"
CALIB="${BATS_TEST_DIRNAME}/../scripts/calibrate-profile.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/routing-budget-XXXXXX")"
    PROF="$TMP/profiles.yml"
    # Local priced ABOVE cloud at normal budget, so only budget pressure can flip it.
    cat > "$PROF" <<'EOF'
claude-haiku-cloud:
  model: claude-haiku-4-5
  ctx: 64k
  reasoning: medium
  cost_in: 1.0
  cost_out: 5.0
qwen3-32b-laptop:
  model: qwen3:32b
  ctx: 64k
  reasoning: medium
  cost_minute: 1.0
EOF
}

teardown() { rm -rf "$TMP"; }

# row <profile> <cache_hit> <mean_wall_ms>
row() {
    printf '{"dispatch_kind":"implementer","profile":"%s","cell_ctx":"64k","cell_reasoning":"medium","dispatches":50,"first_pass_rate":0.85,"failure_rate":0.05,"escalation_rate":0.05,"mean_retries":0.3,"cache_hit_ratio":%s,"mean_wall_clock_ms":%s}' \
        "$1" "$2" "$3"
}

# ── R10: budget pressure -> paid-tier penalty ─────────────────────────────────

@test "routing-budget-hint.sh is executable and --help exits 0" {
    run test -x "$HINT"
    [ "$status" -eq 0 ]
    run bash "$HINT" --help
    [ "$status" -eq 0 ]
}

@test "an untouched budget produces no distortion at all" {
    run bash "$HINT" --used-pct 10 --json
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.cloud_multiplier')" = "1.0" ]
    [ "$(printf '%s' "$output" | jq -r '.hint')" = "normal" ]
}

@test "the multiplier rises monotonically as the budget runs down" {
    prev=""
    for used in 10 60 80 95; do
        run bash "$HINT" --used-pct "$used" --json
        [ "$status" -eq 0 ]
        m="$(printf '%s' "$output" | jq -r '.cloud_multiplier')"
        if [ -n "$prev" ]; then
            run env A="$prev" B="$m" python3 -c "import os;assert float(os.environ['B'])>=float(os.environ['A'])"
            [ "$status" -eq 0 ]
        fi
        prev="$m"
    done
    [ "$prev" = "4.0" ]
}

@test "a nearly-exhausted budget asks for local" {
    run bash "$HINT" --used-pct 95 --json
    [ "$(printf '%s' "$output" | jq -r '.hint')" = "prefer-local" ]
}

@test "an unreadable signal fails OPEN so telemetry gaps cannot distort routing" {
    run env HOME="$TMP" AUTOSPEC_SCRIPTS_DIR="$TMP/none" bash "$HINT" --json --repo-dir "$TMP"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.cloud_multiplier')" = "1.0" ]
}

@test "an explicit hint override wins over the derived band" {
    run env AUTOSPEC_ROUTING_BUDGET_HINT=prefer-local bash "$HINT" --used-pct 5 --json
    [ "$(printf '%s' "$output" | jq -r '.hint')" = "prefer-local" ]
    [ "$(printf '%s' "$output" | jq -r '.cloud_multiplier')" = "4.0" ]
}

@test "the multiplier penalises token-priced profiles but never local ones" {
    # Local profiles are priced in wall clock and consume no token budget, so
    # scaling them under token pressure would be incoherent.
    jq -n --argjson a "$(row qwen3-32b-laptop 0.0 60000)" \
          --argjson b "$(row claude-haiku-cloud 0.8 60000)" '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_ROUTING_CLOUD_MULTIPLIER=1.0 AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" \
        --kind implementer --ctx 64k --reasoning medium \
        --candidates "qwen3-32b-laptop,claude-haiku-cloud" --stats-file "$TMP/s.json"
    base="$output"
    run env AUTOSPEC_ROUTING_CLOUD_MULTIPLIER=4.0 AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" \
        --kind implementer --ctx 64k --reasoning medium \
        --candidates "qwen3-32b-laptop,claude-haiku-cloud" --stats-file "$TMP/s.json"
    scaled="$output"
    q1="$(printf '%s' "$base"   | jq -r '.[]|select(.profile=="qwen3-32b-laptop")|.unit')"
    q2="$(printf '%s' "$scaled" | jq -r '.[]|select(.profile=="qwen3-32b-laptop")|.unit')"
    h1="$(printf '%s' "$base"   | jq -r '.[]|select(.profile=="claude-haiku-cloud")|.unit')"
    h2="$(printf '%s' "$scaled" | jq -r '.[]|select(.profile=="claude-haiku-cloud")|.unit')"
    [ "$q1" = "$q2" ]
    run env A="$h1" B="$h2" python3 -c "import os;assert float(os.environ['B'])>float(os.environ['A'])"
    [ "$status" -eq 0 ]
}

@test "budget pressure shifts the decision from cloud to local" {
    jq -n --argjson a "$(row qwen3-32b-laptop 0.0 60000)" \
          --argjson b "$(row claude-haiku-cloud 0.8 60000)" '[$a,$b]' > "$TMP/s.json"
    run env AUTOSPEC_ROUTING_CLOUD_MULTIPLIER=1.0 AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
        --profiles-file "$PROF" --labels "auto-implement,reasoning:medium,ctx:64k" \
        --stats-file "$TMP/s.json" --print-profile
    [ "$output" = "claude-haiku-cloud" ]
    run env AUTOSPEC_ROUTING_CLOUD_MULTIPLIER=4.0 AUTOSPEC_MODEL_PROFILES="$PROF" bash "$DECIDE" \
        --profiles-file "$PROF" --labels "auto-implement,reasoning:medium,ctx:64k" \
        --stats-file "$TMP/s.json" --print-profile
    [ "$output" = "qwen3-32b-laptop" ]
}

# ── R9: wall-clock ceiling ────────────────────────────────────────────────────

@test "a cheap reliable profile is ineligible when it blows its latency ceiling" {
    # 40 GPU-minutes on an issue a cloud tier finishes in 90s is a throughput
    # regression, not a saving — however good its first-pass rate is.
    printf '  max_wall_clock_ms: 300000\n' >> "$PROF"
    jq -n --argjson a "$(row qwen3-32b-laptop 0.0 2400000)" '[$a]' > "$TMP/slow.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" --kind implementer --ctx 64k \
        --reasoning medium --candidates "qwen3-32b-laptop" --stats-file "$TMP/slow.json"
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.[0].eligible')" = "false" ]
    [[ "$output" == *"exceeds ceiling"* ]]
}

@test "no ceiling configured means latency does not gate" {
    jq -n --argjson a "$(row qwen3-32b-laptop 0.0 2400000)" '[$a]' > "$TMP/slow.json"
    run env AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" --kind implementer --ctx 64k \
        --reasoning medium --candidates "qwen3-32b-laptop" --stats-file "$TMP/slow.json"
    [ "$(printf '%s' "$output" | jq -r '.[0].wall_clock_ceiling_ms')" = "0" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].eligible')" = "true" ]
}

@test "a global ceiling applies to profiles that declare none" {
    jq -n --argjson a "$(row qwen3-32b-laptop 0.0 2400000)" '[$a]' > "$TMP/slow.json"
    run env AUTOSPEC_ROUTING_MAX_WALL_CLOCK_MS=300000 AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" \
        --kind implementer --ctx 64k --reasoning medium \
        --candidates "qwen3-32b-laptop" --stats-file "$TMP/slow.json"
    [ "$(printf '%s' "$output" | jq -r '.[0].eligible')" = "false" ]
}

@test "a per-profile ceiling overrides the global one" {
    printf '  max_wall_clock_ms: 9000000\n' >> "$PROF"
    jq -n --argjson a "$(row qwen3-32b-laptop 0.0 2400000)" '[$a]' > "$TMP/slow.json"
    run env AUTOSPEC_ROUTING_MAX_WALL_CLOCK_MS=300000 AUTOSPEC_MODEL_PROFILES="$PROF" bash "$COST" \
        --kind implementer --ctx 64k --reasoning medium \
        --candidates "qwen3-32b-laptop" --stats-file "$TMP/slow.json"
    [ "$(printf '%s' "$output" | jq -r '.[0].wall_clock_ceiling_ms')" = "9000000" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].eligible')" = "true" ]
}

# ── R11: calibration ──────────────────────────────────────────────────────────

@test "calibrate-profile.sh is executable and --help exits 0" {
    run test -x "$CALIB"
    [ "$status" -eq 0 ]
    run bash "$CALIB" --help
    [ "$status" -eq 0 ]
}

@test "calibration requires a profile" {
    run bash "$CALIB"
    [ "$status" -eq 1 ]
    [[ "$output" == *"--profile is required"* ]]
}

@test "calibration rejects a non-numeric count" {
    run bash "$CALIB" --profile p --model m --count abc
    [ "$status" -eq 1 ]
}

@test "a dry run names the model, replay set, gate and verdict path" {
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" bash "$CALIB" --profile qwen3-32b-laptop \
        --model qwen3:32b --issues 1,2,3 --gate-cmd "true" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"qwen3:32b"* ]]
    [[ "$output" == *"1,2,3"* ]]
    [[ "$output" == *"verdict file"* ]]
}

@test "calibration refuses when no model id can be resolved" {
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" AUTOSPEC_MODEL_PROFILES="$TMP/absent.yml" \
        bash "$CALIB" --profile ghost-profile --issues 1 --gate-cmd "true"
    [ "$status" -eq 3 ]
    [[ "$output" == *"cannot resolve a model id"* ]]
}

@test "a cached verdict for unchanged hardware is reused rather than re-measured" {
    mkdir -p "$TMP/cal"
    fp="$(bash "${BATS_TEST_DIRNAME}/../scripts/discover-model-supply.sh" --fingerprint 2>/dev/null || printf 'unknown')"
    printf '{"profile":"p","model":"m","fingerprint":"%s","attempted":5,"passed":0,"qualified":false}\n' "$fp" \
        > "$TMP/cal/p.$fp.json"
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" bash "$CALIB" --profile p --model m --json
    [ "$status" -eq 0 ]
    # "qualified for zero tiers" is a legitimate result, reported as a clean exit.
    [ "$(printf '%s' "$output" | jq -r '.qualified')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.attempted')" = "5" ]
}

# ── §32/§33: per-role calibration verdicts and bounded exploration ────────────

@test "--calibrate is an alias for --profile" {
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" bash "$CALIB" --calibrate qwen3-32b-laptop \
        --model qwen3:32b --issues 1,2 --gate-cmd "true" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"qwen3-32b-laptop"* ]]
}

@test "--role rejects a name outside the 14-role vocabulary" {
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" bash "$CALIB" --calibrate p --model m \
        --role wizard --issues 1 --gate-cmd "true" --dry-run
    [ "$status" -eq 1 ]
    [[ "$output" == *"role"* ]]
}

@test "--role accepts every one of the 14 roles" {
    for r in orchestrator planner architect test_planner implementer code_reviewer \
             test_reviewer qa_verifier documentation_writer documentation_reviewer \
             ui_ux_reviewer security_reviewer researcher advisor; do
        run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" bash "$CALIB" --calibrate p --model m \
            --role "$r" --issues 1 --gate-cmd "true" --dry-run
        [ "$status" -eq 0 ]
    done
}

@test "--calibrate P --role implementer writes a role-scoped qualified verdict" {
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" \
        AUTOSPEC_MODEL_CAPABILITY="$TMP/absent-capability.json" \
        AUTOSPEC_ROUTING_LEDGER="$TMP/ledger.jsonl" \
        bash "$CALIB" --calibrate qwen3-32b-laptop --model qwen3:32b \
        --role implementer --issues 1 --gate-cmd "true" --json
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.role')" = "implementer" ]
    [ "$(printf '%s' "$output" | jq -r '.qualified | type')" = "boolean" ]
    verdict="$(ls "$TMP"/cal/qwen3-32b-laptop.*.implementer.json)"
    run test -f "$verdict"
    [ "$status" -eq 0 ]
}

@test "a role verdict does not collide with the profile-level verdict path" {
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" \
        AUTOSPEC_MODEL_CAPABILITY="$TMP/absent-capability.json" \
        AUTOSPEC_ROUTING_LEDGER="$TMP/ledger.jsonl" \
        bash "$CALIB" --calibrate p --model m --role planner --issues 1 --gate-cmd "true"
    [ "$status" -eq 0 ]
    run bash -c "ls '$TMP'/cal/p.*.json | grep -c planner"
    [ "$output" = "1" ]
}

@test "partial qualification is reported as a clean exit, not an error" {
    mkdir -p "$TMP/cal"
    fp="$(bash "${BATS_TEST_DIRNAME}/../scripts/discover-model-supply.sh" --fingerprint 2>/dev/null || printf 'unknown')"
    printf '{"profile":"p","model":"m","role":"planner","fingerprint":"%s","attempted":5,"passed":2,"qualified":false}\n' "$fp" \
        > "$TMP/cal/p.$fp.planner.json"
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" bash "$CALIB" --calibrate p --model m \
        --role planner --json
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.qualified')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.passed')" = "2" ]
}

@test "exploration refuses security_reviewer" {
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" bash "$CALIB" --calibrate p --model m \
        --role security_reviewer --exploration-budget 3 --issues 1 --gate-cmd "true"
    [ "$status" -eq 4 ]
    [[ "$output" == *"reason=role_forbidden"* ]]
}

@test "exploration refuses every independent-review role" {
    for r in code_reviewer test_reviewer documentation_reviewer ui_ux_reviewer qa_verifier; do
        run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" bash "$CALIB" --calibrate p --model m \
            --role "$r" --exploration-budget 3 --issues 1 --gate-cmd "true"
        [ "$status" -eq 4 ]
    done
}

@test "exploration permits an implementer role" {
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" bash "$CALIB" --calibrate p --model m \
        --role implementer --exploration-budget 3 --issues 1,2 --gate-cmd "true" --dry-run
    [ "$status" -eq 0 ]
}

@test "--exploration-budget bounds the replay set" {
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" bash "$CALIB" --calibrate p --model m \
        --role implementer --exploration-budget 2 --issues 1,2,3,4,5 --gate-cmd "true" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"replay issues: 1,2"* ]]
}

@test "--exploration-budget rejects a non-numeric value" {
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" bash "$CALIB" --calibrate p --model m \
        --role implementer --exploration-budget many --issues 1 --dry-run
    [ "$status" -eq 1 ]
}

@test "an exploration budget of zero refuses to explore at all" {
    run env AUTOSPEC_CALIBRATION_DIR="$TMP/cal" bash "$CALIB" --calibrate p --model m \
        --role implementer --exploration-budget 0 --issues 1,2 --gate-cmd "true"
    [ "$status" -eq 4 ]
    [[ "$output" == *"reason="* ]]
}

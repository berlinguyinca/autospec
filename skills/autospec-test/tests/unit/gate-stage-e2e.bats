#!/usr/bin/env bats
# skills/autospec-test/tests/unit/gate-stage-e2e.bats
#
# TDD tests for Phase 3: Stage 2 E2E gate components
#   - playwright-config-resolver.mjs
#   - forbidden-url-check.mjs
#   - network-intercept-inject.mjs
#   - ui-crawler.mjs (static mode)
#   - behavior-taxonomy-check.mjs
#   - findings-generator.mjs
#   - gate-stage-e2e.sh (smoke)

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"
    FIXTURES_DIR="$REPO_ROOT/skills/autospec-test/tests/fixtures"
    PW_CONFIGS="$FIXTURES_DIR/playwright-configs"
    STATIC_SITE="$FIXTURES_DIR/static-site"

    TEST_TMPDIR="$(mktemp -d /tmp/autospec-e2e-bats-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# ── playwright-config-resolver.mjs ────────────────────────────────────────────

@test "config-resolver: finds playwright.config.js and extracts baseURL" {
    run node "$SCRIPTS_DIR/playwright-config-resolver.mjs" "$PW_CONFIGS"
    [ "$status" -eq 0 ]
    local baseURL
    baseURL=$(printf '%s' "$output" | jq -r '.baseURL')
    [[ "$baseURL" == "http://localhost"* ]]
}

@test "config-resolver: emits valid JSON with required fields" {
    run node "$SCRIPTS_DIR/playwright-config-resolver.mjs" "$PW_CONFIGS"
    [ "$status" -eq 0 ]
    run bash -c "printf '%s' '$output' | jq -e '.configPath and (.baseURL != null or .baseURL == null)'"
    [ "$status" -eq 0 ]
}

@test "config-resolver: missing repo exits 1" {
    run node "$SCRIPTS_DIR/playwright-config-resolver.mjs" "/nonexistent/path/$$"
    [ "$status" -eq 1 ]
}

@test "config-resolver: missing arg exits 1" {
    run node "$SCRIPTS_DIR/playwright-config-resolver.mjs"
    [ "$status" -eq 1 ]
}

@test "config-resolver: repo without playwright.config still emits JSON" {
    run node "$SCRIPTS_DIR/playwright-config-resolver.mjs" "$TEST_TMPDIR"
    [ "$status" -eq 0 ]
    local configPath
    configPath=$(printf '%s' "$output" | jq -r '.configPath')
    [ "$configPath" = "null" ]
}

# ── forbidden-url-check.mjs ───────────────────────────────────────────────────

make_config_json() {
    local base_url="${1:-http://localhost:3000}"
    jq -n --arg url "$base_url" '{"baseURL":$url,"useBaseURL":$url,"webServerURL":$url}'
}

make_contract_json() {
    local pattern="${1:-^https?://prod\\.example\\.com}"
    jq -n --arg p "$pattern" '{"e2e":{"forbidden_url_patterns":[$p]}}'
}

@test "forbidden-url-check: no violation exits 0 for safe URL" {
    local config_file="$TEST_TMPDIR/config.json"
    local contract_file="$TEST_TMPDIR/contract.json"
    make_config_json "http://localhost:3000" > "$config_file"
    make_contract_json "^https?://prod\\.example\\.com" > "$contract_file"
    run node "$SCRIPTS_DIR/forbidden-url-check.mjs" "$config_file" "$contract_file"
    [ "$status" -eq 0 ]
}

@test "forbidden-url-check: violation exits 2 when URL matches pattern" {
    local config_file="$TEST_TMPDIR/config.json"
    local contract_file="$TEST_TMPDIR/contract.json"
    make_config_json "https://prod.example.com" > "$config_file"
    make_contract_json "^https?://prod\\.example\\.com" > "$contract_file"
    run node "$SCRIPTS_DIR/forbidden-url-check.mjs" "$config_file" "$contract_file"
    [ "$status" -eq 2 ]
}

@test "forbidden-url-check: violation output has passed=false" {
    local config_file="$TEST_TMPDIR/config.json"
    local contract_file="$TEST_TMPDIR/contract.json"
    make_config_json "https://prod.example.com" > "$config_file"
    make_contract_json "^https?://prod\\.example\\.com" > "$contract_file"
    local output
    output=$(node "$SCRIPTS_DIR/forbidden-url-check.mjs" "$config_file" "$contract_file" 2>/dev/null || true)
    local passed
    passed=$(printf '%s' "$output" | jq -r '.passed')
    [ "$passed" = "false" ]
}

@test "forbidden-url-check: violation output has non-empty violations array" {
    local config_file="$TEST_TMPDIR/config.json"
    local contract_file="$TEST_TMPDIR/contract.json"
    make_config_json "https://prod.example.com" > "$config_file"
    make_contract_json "^https?://prod\\.example\\.com" > "$contract_file"
    local output
    output=$(node "$SCRIPTS_DIR/forbidden-url-check.mjs" "$config_file" "$contract_file" 2>/dev/null || true)
    local count
    count=$(printf '%s' "$output" | jq '.violations | length')
    [ "$count" -gt 0 ]
}

@test "forbidden-url-check: passes when pattern list is empty with ack" {
    local config_file="$TEST_TMPDIR/config.json"
    local contract_file="$TEST_TMPDIR/contract.json"
    make_config_json "https://prod.example.com" > "$config_file"
    jq -n '{"e2e":{"forbidden_url_patterns":[],"forbidden_url_patterns_intentionally_empty":true}}' > "$contract_file"
    run node "$SCRIPTS_DIR/forbidden-url-check.mjs" "$config_file" "$contract_file"
    [ "$status" -eq 0 ]
}

@test "forbidden-url-check: missing args exits 1" {
    run node "$SCRIPTS_DIR/forbidden-url-check.mjs"
    [ "$status" -eq 1 ]
}

# ── network-intercept-inject.mjs ──────────────────────────────────────────────

@test "network-intercept-inject: writes global-setup-autospec.ts" {
    local contract_file="$TEST_TMPDIR/contract.json"
    jq -n '{"e2e":{"forbidden_url_patterns":["^https?://prod\\.example\\.com"]}}' > "$contract_file"
    run node "$SCRIPTS_DIR/network-intercept-inject.mjs" "$TEST_TMPDIR" "$contract_file"
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMPDIR/playwright/global-setup-autospec.ts" ]
}

@test "network-intercept-inject: idempotent on second run" {
    local contract_file="$TEST_TMPDIR/contract.json"
    jq -n '{"e2e":{"forbidden_url_patterns":["^https?://prod\\.example\\.com"]}}' > "$contract_file"
    node "$SCRIPTS_DIR/network-intercept-inject.mjs" "$TEST_TMPDIR" "$contract_file" >/dev/null 2>/dev/null
    local output1
    output1=$(cat "$TEST_TMPDIR/playwright/global-setup-autospec.ts")
    node "$SCRIPTS_DIR/network-intercept-inject.mjs" "$TEST_TMPDIR" "$contract_file" >/dev/null 2>/dev/null
    local output2
    output2=$(cat "$TEST_TMPDIR/playwright/global-setup-autospec.ts")
    [ "$output1" = "$output2" ]
}

@test "network-intercept-inject: exits 2 when patterns empty without ack" {
    local contract_file="$TEST_TMPDIR/contract.json"
    jq -n '{"e2e":{"forbidden_url_patterns":[]}}' > "$contract_file"
    run node "$SCRIPTS_DIR/network-intercept-inject.mjs" "$TEST_TMPDIR" "$contract_file"
    [ "$status" -eq 2 ]
}

@test "network-intercept-inject: output JSON has globalSetupPath" {
    local contract_file="$TEST_TMPDIR/contract.json"
    jq -n '{"e2e":{"forbidden_url_patterns":["^https?://prod\\.example\\.com"]}}' > "$contract_file"
    local output
    output=$(node "$SCRIPTS_DIR/network-intercept-inject.mjs" "$TEST_TMPDIR" "$contract_file" 2>/dev/null)
    local gsp
    gsp=$(printf '%s' "$output" | jq -r '.globalSetupPath')
    [[ "$gsp" == *"global-setup-autospec.ts"* ]]
}

# ── ui-crawler.mjs (static mode) ──────────────────────────────────────────────

@test "ui-crawler: crawls static site and finds elements" {
    run node "$SCRIPTS_DIR/ui-crawler.mjs" "$STATIC_SITE"
    [ "$status" -eq 0 ]
    local count
    count=$(printf '%s' "$output" | jq '.elements_found')
    [ "$count" -gt 0 ]
}

@test "ui-crawler: prefers data-testid selectors over role+name" {
    run node "$SCRIPTS_DIR/ui-crawler.mjs" "$STATIC_SITE"
    [ "$status" -eq 0 ]
    local strategies
    strategies=$(printf '%s' "$output" | jq -r '[.elements[].strategy] | join(",")')
    [[ "$strategies" == *"data-testid"* ]]
}

@test "ui-crawler: caps at MAX_ROUTES" {
    run bash -c "MAX_ROUTES=1 node '$SCRIPTS_DIR/ui-crawler.mjs' '$STATIC_SITE'"
    [ "$status" -eq 0 ]
    local routes
    routes=$(printf '%s' "$output" | jq '.routes_found')
    [ "$routes" -le 1 ]
}

@test "ui-crawler: emits valid JSON with routes and elements arrays" {
    run node "$SCRIPTS_DIR/ui-crawler.mjs" "$STATIC_SITE"
    [ "$status" -eq 0 ]
    run bash -c "printf '%s' '$output' | jq -e '.routes and .elements and .routes_found >= 0'"
    [ "$status" -eq 0 ]
}

@test "ui-crawler: missing arg exits 1" {
    run node "$SCRIPTS_DIR/ui-crawler.mjs"
    [ "$status" -eq 1 ]
}

# ── behavior-taxonomy-check.mjs ───────────────────────────────────────────────

@test "behavior-taxonomy-check: empty test results all missing exits 2" {
    local contract_file="$TEST_TMPDIR/contract.json"
    # Contract with all 9 behavior categories declared
    jq -n '{"e2e":{"coverage_thresholds":{"behavior_categories":["sort","scroll","upload"]}}}' > "$contract_file"
    run node "$SCRIPTS_DIR/behavior-taxonomy-check.mjs" "$TEST_TMPDIR/nonexistent" "$contract_file"
    [ "$status" -eq 2 ]
    local missing
    missing=$(printf '%s' "$output" | jq '.missing | length')
    [ "$missing" -gt 0 ]
}

@test "behavior-taxonomy-check: annotation in trace marks category as passing" {
    local contract_file="$TEST_TMPDIR/contract.json"
    jq -n '{"e2e":{"coverage_thresholds":{"behavior_categories":["sort"]}}}' > "$contract_file"
    # Create a fake trace file with annotation
    mkdir -p "$TEST_TMPDIR/test-results"
    printf 'category:sort test passed\n' > "$TEST_TMPDIR/test-results/trace.txt"
    run node "$SCRIPTS_DIR/behavior-taxonomy-check.mjs" "$TEST_TMPDIR/test-results" "$contract_file"
    [ "$status" -eq 0 ]
    local passing
    passing=$(printf '%s' "$output" | jq -r '.passing | join(",")')
    [[ "$passing" == *"sort"* ]]
}

@test "behavior-taxonomy-check: primitive in trace marks category as passing" {
    local contract_file="$TEST_TMPDIR/contract.json"
    jq -n '{"e2e":{"coverage_thresholds":{"behavior_categories":["scroll"]}}}' > "$contract_file"
    mkdir -p "$TEST_TMPDIR/test-results"
    printf 'action: scroll element at position 100\n' > "$TEST_TMPDIR/test-results/trace.txt"
    run node "$SCRIPTS_DIR/behavior-taxonomy-check.mjs" "$TEST_TMPDIR/test-results" "$contract_file"
    [ "$status" -eq 0 ]
}

@test "behavior-taxonomy-check: output has required keys" {
    local contract_file="$TEST_TMPDIR/contract.json"
    jq -n '{"e2e":{"coverage_thresholds":{"behavior_categories":["sort"]}}}' > "$contract_file"
    run node "$SCRIPTS_DIR/behavior-taxonomy-check.mjs" "$TEST_TMPDIR" "$contract_file"
    run bash -c "printf '%s' '$output' | jq -e '.passed != null and .missing and .passing'"
    [ "$status" -eq 0 ]
}

@test "behavior-taxonomy-check: missing args exits 1" {
    run node "$SCRIPTS_DIR/behavior-taxonomy-check.mjs"
    [ "$status" -eq 1 ]
}

# ── findings-generator.mjs ────────────────────────────────────────────────────

@test "findings-generator: writes .autospec/test-findings.md" {
    local gate_file="$TEST_TMPDIR/gate.json"
    jq -n '{"passed":false,"stage":"e2e","metrics":{"unit":{"missing_function_tests":["foo","bar"]}}}' > "$gate_file"
    run node "$SCRIPTS_DIR/findings-generator.mjs" "$gate_file" "$TEST_TMPDIR"
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMPDIR/.autospec/test-findings.md" ]
}

@test "findings-generator: idempotent on identical inputs" {
    local gate_file="$TEST_TMPDIR/gate.json"
    jq -n '{"passed":true,"stage":"e2e"}' > "$gate_file"
    node "$SCRIPTS_DIR/findings-generator.mjs" "$gate_file" "$TEST_TMPDIR" >/dev/null 2>/dev/null
    local hash1
    hash1=$(node "$SCRIPTS_DIR/findings-generator.mjs" "$gate_file" "$TEST_TMPDIR" 2>/dev/null | jq -r '.hash')
    local output2
    output2=$(node "$SCRIPTS_DIR/findings-generator.mjs" "$gate_file" "$TEST_TMPDIR" 2>/dev/null)
    local idempotent
    idempotent=$(printf '%s' "$output2" | jq -r '.idempotent')
    [ "$idempotent" = "true" ]
}

@test "findings-generator: exits 0 even on missing gate file (non-blocking)" {
    run node "$SCRIPTS_DIR/findings-generator.mjs" "/nonexistent/gate.json" "$TEST_TMPDIR"
    [ "$status" -eq 0 ]
}

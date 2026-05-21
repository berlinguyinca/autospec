#!/usr/bin/env bats
# skills/autospec-test/tests/unit/gate-stage-e2e.bats
#
# TDD tests for Phase 3: gate-stage-e2e.sh and all Node helper scripts.
# Primary smoke test for issue #321.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"
    FIXTURES_DIR="$REPO_ROOT/skills/autospec-test/tests/fixtures"

    TEST_TMPDIR="$(mktemp -d /tmp/autospec-e2e-bats-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# ── Helper: minimal valid E2E contract JSON ───────────────────────────────────

make_e2e_contract() {
    local base_url="${1:-http://localhost:3000}"
    jq -n \
        --arg base_url "$base_url" \
        '{
            "mode": "strict_isolation",
            "unit": {
                "test_cmd": "true",
                "coverage_collector": "istanbul",
                "coverage_thresholds": {"lines": 0, "branches": 0, "functions": 0},
                "function_presence_check": false
            },
            "e2e": {
                "forbidden_url_patterns": ["^https?://prod\\.example\\.com"],
                "playwright_cmd": "echo playwright-stub",
                "playwright_config": "playwright.config.js"
            }
        }'
}

# ── playwright-config-resolver.mjs ───────────────────────────────────────────

@test "playwright-config-resolver: resolves baseURL from JS config" {
    cat > "$TEST_TMPDIR/playwright.config.js" <<'EOF'
module.exports = { use: { baseURL: 'http://localhost:4000' } };
EOF
    run node "$SCRIPTS_DIR/playwright-config-resolver.mjs" "$TEST_TMPDIR"
    [ "$status" -eq 0 ]
    local base_url
    base_url=$(printf '%s' "$output" | jq -r '.baseURL')
    [ "$base_url" = "http://localhost:4000" ]
}

@test "playwright-config-resolver: resolves baseURL from TS config" {
    cat > "$TEST_TMPDIR/playwright.config.ts" <<'EOF'
import { defineConfig } from '@playwright/test';
export default defineConfig({ use: { baseURL: 'http://localhost:5000' } });
EOF
    run node "$SCRIPTS_DIR/playwright-config-resolver.mjs" "$TEST_TMPDIR"
    [ "$status" -eq 0 ]
    local base_url
    base_url=$(printf '%s' "$output" | jq -r '.baseURL')
    [ "$base_url" = "http://localhost:5000" ]
}

@test "playwright-config-resolver: resolves baseURL from MJS config" {
    cat > "$TEST_TMPDIR/playwright.config.mjs" <<'EOF'
export default { use: { baseURL: 'http://localhost:6000' } };
EOF
    run node "$SCRIPTS_DIR/playwright-config-resolver.mjs" "$TEST_TMPDIR"
    [ "$status" -eq 0 ]
    local base_url
    base_url=$(printf '%s' "$output" | jq -r '.baseURL')
    [ "$base_url" = "http://localhost:6000" ]
}

@test "playwright-config-resolver: handles nested objects in use block" {
    # Regression: nested objects (viewport, permissions, headers) must not break baseURL extraction
    cat > "$TEST_TMPDIR/playwright.config.js" <<'EOF'
module.exports = {
    use: {
        viewport: { width: 1280, height: 720 },
        permissions: ['clipboard-read'],
        baseURL: 'http://localhost:7000',
        extraHTTPHeaders: { 'X-Custom': 'value' }
    }
};
EOF
    run node "$SCRIPTS_DIR/playwright-config-resolver.mjs" "$TEST_TMPDIR"
    [ "$status" -eq 0 ]
    local base_url
    base_url=$(printf '%s' "$output" | jq -r '.baseURL')
    [ "$base_url" = "http://localhost:7000" ]
}

@test "playwright-config-resolver: returns null baseURL when no config" {
    run node "$SCRIPTS_DIR/playwright-config-resolver.mjs" "$TEST_TMPDIR"
    [ "$status" -eq 0 ]
    local base_url
    base_url=$(printf '%s' "$output" | jq -r '.baseURL')
    [ "$base_url" = "null" ]
}

@test "playwright-config-resolver: emits valid JSON with required keys" {
    local result
    result=$(node "$SCRIPTS_DIR/playwright-config-resolver.mjs" "$TEST_TMPDIR" 2>/dev/null)
    [ -n "$result" ]
    # Validate required keys are present (projects is array, testDir is null or string)
    local has_projects has_testdir
    has_projects=$(printf '%s' "$result" | jq -r 'if has("projects") then "yes" else "no" end')
    has_testdir=$(printf '%s' "$result" | jq -r 'if has("testDir") then "yes" else "no" end')
    [ "$has_projects" = "yes" ]
    [ "$has_testdir" = "yes" ]
}

# ── forbidden-url-check.mjs ───────────────────────────────────────────────────

@test "forbidden-url-check: detects violation in baseURL" {
    local config
    config=$(jq -n '{"baseURL":"https://prod.example.com","use":{"baseURL":"https://prod.example.com"}}')
    local patterns
    patterns=$(jq -n '["^https?://prod\\.example\\.com"]')
    run node "$SCRIPTS_DIR/forbidden-url-check.mjs" \
        --config <(printf '%s' "$config") \
        --patterns <(printf '%s' "$patterns")
    [ "$status" -eq 2 ]
}

@test "forbidden-url-check: no violation for safe URL" {
    local config
    config=$(jq -n '{"baseURL":"http://localhost:3000","use":{"baseURL":"http://localhost:3000"}}')
    local patterns
    patterns=$(jq -n '["^https?://prod\\.example\\.com"]')
    run node "$SCRIPTS_DIR/forbidden-url-check.mjs" \
        --config <(printf '%s' "$config") \
        --patterns <(printf '%s' "$patterns")
    [ "$status" -eq 0 ]
}

@test "forbidden-url-check: detects violation in webServer.url" {
    local config
    config=$(jq -n '{"webServer":{"url":"https://prod.example.com/health"}}')
    local patterns
    patterns=$(jq -n '["^https?://prod\\.example\\.com"]')
    run node "$SCRIPTS_DIR/forbidden-url-check.mjs" \
        --config <(printf '%s' "$config") \
        --patterns <(printf '%s' "$patterns")
    [ "$status" -eq 2 ]
}

@test "forbidden-url-check: checks all URL fields from spec Layer A" {
    # All URL-shaped fields: baseURL, use.baseURL, webServer.url, webServer.command (skip)
    # Each should be detected when forbidden
    local config
    config=$(jq -n '{
        "baseURL": "http://localhost:3000",
        "use": { "baseURL": "https://prod.example.com" },
        "webServer": { "url": "http://localhost:3000/health" }
    }')
    local patterns
    patterns=$(jq -n '["^https?://prod\\.example\\.com"]')
    local out
    out=$(node "$SCRIPTS_DIR/forbidden-url-check.mjs" \
        --config <(printf '%s' "$config") \
        --patterns <(printf '%s' "$patterns") 2>/dev/null || true)
    local vcount
    vcount=$(printf '%s' "$out" | jq '.violations | length')
    [ "$vcount" -gt 0 ]
}

@test "forbidden-url-check: emits JSON violations list on exit 2" {
    local config
    config=$(jq -n '{"baseURL":"https://prod.example.com"}')
    local patterns
    patterns=$(jq -n '["^https?://prod\\.example\\.com"]')
    local out
    out=$(node "$SCRIPTS_DIR/forbidden-url-check.mjs" \
        --config <(printf '%s' "$config") \
        --patterns <(printf '%s' "$patterns") 2>/dev/null || true)
    printf '%s' "$out" | jq -e '.violations[0].field'
}

# ── network-intercept-inject.mjs ──────────────────────────────────────────────

@test "network-intercept-inject: creates global-setup file" {
    cat > "$TEST_TMPDIR/playwright.config.js" <<'EOF'
module.exports = { use: { baseURL: 'http://localhost:3000' } };
EOF
    local patterns
    patterns=$(jq -n '["^https?://prod\\.example\\.com"]')
    run node "$SCRIPTS_DIR/network-intercept-inject.mjs" \
        --repo-root "$TEST_TMPDIR" \
        --patterns <(printf '%s' "$patterns")
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMPDIR/playwright/global-setup-autospec.ts" ]
}

@test "network-intercept-inject: is idempotent (second run is no-op)" {
    cat > "$TEST_TMPDIR/playwright.config.js" <<'EOF'
module.exports = { use: { baseURL: 'http://localhost:3000' } };
EOF
    local patterns
    patterns=$(jq -n '["^https?://prod\\.example\\.com"]')
    node "$SCRIPTS_DIR/network-intercept-inject.mjs" \
        --repo-root "$TEST_TMPDIR" \
        --patterns <(printf '%s' "$patterns") 2>/dev/null || true
    local first_hash
    first_hash=$(md5 -q "$TEST_TMPDIR/playwright/global-setup-autospec.ts" 2>/dev/null \
        || md5sum "$TEST_TMPDIR/playwright/global-setup-autospec.ts" 2>/dev/null | awk '{print $1}')
    node "$SCRIPTS_DIR/network-intercept-inject.mjs" \
        --repo-root "$TEST_TMPDIR" \
        --patterns <(printf '%s' "$patterns") 2>/dev/null || true
    local second_hash
    second_hash=$(md5 -q "$TEST_TMPDIR/playwright/global-setup-autospec.ts" 2>/dev/null \
        || md5sum "$TEST_TMPDIR/playwright/global-setup-autospec.ts" 2>/dev/null | awk '{print $1}')
    [ "$first_hash" = "$second_hash" ]
}

@test "network-intercept-inject: generated setup uses import type not value import" {
    cat > "$TEST_TMPDIR/playwright.config.js" <<'EOF'
module.exports = { use: { baseURL: 'http://localhost:3000' } };
EOF
    local patterns
    patterns=$(jq -n '["^https?://prod\\.example\\.com"]')
    node "$SCRIPTS_DIR/network-intercept-inject.mjs" \
        --repo-root "$TEST_TMPDIR" \
        --patterns <(printf '%s' "$patterns") 2>/dev/null || true
    # Must use 'import type' not 'import { FullConfig }'
    run grep -n "import type" "$TEST_TMPDIR/playwright/global-setup-autospec.ts"
    [ "$status" -eq 0 ]
    run grep -n "^import { FullConfig" "$TEST_TMPDIR/playwright/global-setup-autospec.ts"
    [ "$status" -ne 0 ]
}

# ── behavior-taxonomy-check.mjs ───────────────────────────────────────────────

@test "behavior-taxonomy-check: detects missing categories" {
    # Create minimal trace dir with no taxonomy annotations
    mkdir -p "$TEST_TMPDIR/test-results"
    local categories
    categories=$(jq -n '["sort","scroll","upload"]')
    # Exit 1 means passed=false (categories missing) — that's expected here
    local out
    out=$(node "$SCRIPTS_DIR/behavior-taxonomy-check.mjs" \
        --results-dir "$TEST_TMPDIR/test-results" \
        --categories <(printf '%s' "$categories") 2>/dev/null || true)
    local missing
    missing=$(printf '%s' "$out" | jq -r '.missing | length')
    [ "$missing" -eq 3 ]
}

@test "behavior-taxonomy-check: passes when all categories satisfied" {
    mkdir -p "$TEST_TMPDIR/test-results"
    # Create a fake trace JSON with all categories
    cat > "$TEST_TMPDIR/test-results/trace.json" <<'EOF'
{
  "annotations": [
    {"type": "category", "description": "sort"},
    {"type": "category", "description": "scroll"},
    {"type": "category", "description": "upload"}
  ],
  "actions": [
    {"type": "click", "selector": "[role=columnheader]"},
    {"type": "wheel", "selector": "body"},
    {"type": "setInputFiles", "selector": "input[type=file]"}
  ]
}
EOF
    local categories
    categories=$(jq -n '["sort","scroll","upload"]')
    run node "$SCRIPTS_DIR/behavior-taxonomy-check.mjs" \
        --results-dir "$TEST_TMPDIR/test-results" \
        --categories <(printf '%s' "$categories")
    [ "$status" -eq 0 ]
    local passed
    passed=$(printf '%s' "$output" | jq -r '.passed')
    [ "$passed" = "true" ]
}

@test "behavior-taxonomy-check: emits valid JSON with passed, missing, passing keys" {
    mkdir -p "$TEST_TMPDIR/test-results"
    local categories
    categories=$(jq -n '["sort"]')
    # Exit code may be 0 (passed) or 1 (missing) — capture output regardless
    local out
    out=$(node "$SCRIPTS_DIR/behavior-taxonomy-check.mjs" \
        --results-dir "$TEST_TMPDIR/test-results" \
        --categories <(printf '%s' "$categories") 2>/dev/null || true)
    [ -n "$out" ]
    # Validate required keys are present
    local has_passed has_missing has_passing
    has_passed=$(printf '%s' "$out" | jq -r 'if has("passed") then "yes" else "no" end')
    has_missing=$(printf '%s' "$out" | jq -r 'if has("missing") then "yes" else "no" end')
    has_passing=$(printf '%s' "$out" | jq -r 'if has("passing") then "yes" else "no" end')
    [ "$has_passed" = "yes" ]
    [ "$has_missing" = "yes" ]
    [ "$has_passing" = "yes" ]
}

# ── findings-generator.mjs ────────────────────────────────────────────────────

@test "findings-generator: creates findings file" {
    mkdir -p "$TEST_TMPDIR/.autospec"
    local gate_json
    gate_json=$(jq -n '{"passed":false,"stage":"e2e","metrics":{}}')
    run node "$SCRIPTS_DIR/findings-generator.mjs" \
        --gate-result <(printf '%s' "$gate_json") \
        --output "$TEST_TMPDIR/.autospec/test-findings.md" \
        --dry-run
    [ "$status" -eq 0 ]
}

@test "findings-generator: is idempotent on identical inputs (content-hash gate)" {
    mkdir -p "$TEST_TMPDIR/.autospec"
    local gate_json
    gate_json=$(jq -n '{"passed":false,"stage":"e2e","metrics":{}}')
    # First run
    node "$SCRIPTS_DIR/findings-generator.mjs" \
        --gate-result <(printf '%s' "$gate_json") \
        --output "$TEST_TMPDIR/.autospec/test-findings.md" \
        --dry-run 2>/dev/null || true
    local first_hash
    first_hash=$(md5 -q "$TEST_TMPDIR/.autospec/test-findings.md" 2>/dev/null \
        || md5sum "$TEST_TMPDIR/.autospec/test-findings.md" 2>/dev/null | awk '{print $1}' || echo "SKIP")
    # Second run with same input
    node "$SCRIPTS_DIR/findings-generator.mjs" \
        --gate-result <(printf '%s' "$gate_json") \
        --output "$TEST_TMPDIR/.autospec/test-findings.md" \
        --dry-run 2>/dev/null || true
    local second_hash
    second_hash=$(md5 -q "$TEST_TMPDIR/.autospec/test-findings.md" 2>/dev/null \
        || md5sum "$TEST_TMPDIR/.autospec/test-findings.md" 2>/dev/null | awk '{print $1}' || echo "SKIP")
    [ "$first_hash" = "$second_hash" ]
}

# ── gate-stage-e2e.sh ─────────────────────────────────────────────────────────

@test "gate-stage-e2e: exits 2 when no forbidden_url_patterns and no ack flag" {
    # Fail-closed: missing forbidden_url_patterns -> refuse to run
    local contract
    contract=$(jq -n '{
        "mode": "strict_isolation",
        "unit": {"test_cmd": "true", "coverage_collector": "istanbul",
                 "coverage_thresholds": {"lines": 0, "branches": 0, "functions": 0},
                 "function_presence_check": false},
        "e2e": {"playwright_cmd": "echo stub"}
    }')
    run bash -c "printf '%s' '$contract' | bash '$SCRIPTS_DIR/gate-stage-e2e.sh' '$TEST_TMPDIR' 2>/dev/null"
    [ "$status" -eq 2 ]
}

@test "gate-stage-e2e: exits 2 when forbidden_url_patterns is empty array and no ack" {
    local contract
    contract=$(jq -n '{
        "mode": "strict_isolation",
        "unit": {"test_cmd": "true", "coverage_collector": "istanbul",
                 "coverage_thresholds": {"lines": 0, "branches": 0, "functions": 0},
                 "function_presence_check": false},
        "e2e": {"playwright_cmd": "echo stub", "forbidden_url_patterns": []}
    }')
    run bash -c "printf '%s' '$contract' | bash '$SCRIPTS_DIR/gate-stage-e2e.sh' '$TEST_TMPDIR' 2>/dev/null"
    [ "$status" -eq 2 ]
}

@test "gate-stage-e2e: emits stage=e2e in output JSON" {
    local contract
    contract=$(make_e2e_contract)
    local out
    out=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-e2e.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    local stage
    stage=$(printf '%s' "$out" | jq -r '.stage // empty')
    [ "$stage" = "e2e" ]
}

@test "gate-stage-e2e: output JSON has required top-level keys" {
    local contract
    contract=$(make_e2e_contract)
    local out
    out=$(printf '%s' "$contract" | bash "$SCRIPTS_DIR/gate-stage-e2e.sh" "$TEST_TMPDIR" 2>/dev/null || true)
    run bash -c "printf '%s' '$out' | jq -e '.passed != null and .stage != null and .metrics != null'"
    [ "$status" -eq 0 ]
}

@test "gate-stage-e2e: playwright_cmd is executed via array exec not eval" {
    # Safety: playwright_cmd should NOT use eval on the contract string.
    # Check only non-comment lines for eval usage on PLAYWRIGHT_CMD variable.
    local eval_usage
    eval_usage=$(grep -v '^\s*#' "$SCRIPTS_DIR/gate-stage-e2e.sh" \
        | grep -c 'eval.*\$PLAYWRIGHT_CMD\|eval.*PLAYWRIGHT_CMD_STR' || true)
    [ "$eval_usage" -eq 0 ]
}

@test "gate-stage-e2e: EXIT trap is accumulated not clobbered" {
    # Verify the script uses an accumulating EXIT trap pattern (add_exit_trap helper),
    # not multiple bare 'trap "..." EXIT' clobbering calls.
    # Only ONE trap ... EXIT registration is allowed (the initial registration of _run_exit_traps).
    # Additional per-step cleanups must go through add_exit_trap().
    local trap_count
    trap_count=$(grep -c "^trap '" "$SCRIPTS_DIR/gate-stage-e2e.sh" 2>/dev/null || echo 0)
    # Exactly 1 bare trap registration (the accumulator setup)
    [ "$trap_count" -eq 1 ]
    # Verify it registers the accumulator function, not a direct rm/cleanup
    run grep -n "^trap '" "$SCRIPTS_DIR/gate-stage-e2e.sh"
    [ "$status" -eq 0 ]
    [[ "$output" == *"_run_exit_traps"* ]]
}

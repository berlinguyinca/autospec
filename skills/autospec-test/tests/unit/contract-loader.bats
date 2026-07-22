#!/usr/bin/env bats
# skills/autospec-test/tests/unit/contract-loader.bats
#
# TDD tests for the Phase 1 contract loader:
#   - load-contract.sh (load_contract)
#   - autodetect.sh
#   - validate-contract.sh
#   - schemas/autospec-test-contract.schema.json
#
# Exit code contract:
#   0 = ok
#   1 = fatal error
#   2 = refuse-to-run (operator-actionable)

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"
    FIXTURES_DIR="$REPO_ROOT/skills/autospec-test/tests/fixtures/contracts"
    SCHEMA="$REPO_ROOT/schemas/autospec-test-contract.schema.json"

    # Create a temp dir for per-test fake repos
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-test-bats-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# ── Helper: make a minimal fake repo with a given contract file ───────────────
make_fake_repo() {
    local contract_src="$1"
    local fake_repo="$TEST_TMPDIR/repo"
    mkdir -p "$fake_repo/.autospec"
    cp "$contract_src" "$fake_repo/.autospec/test.yml"
    printf '%s\n' "$fake_repo"
}

# ── Schema compilation ────────────────────────────────────────────────────────

@test "schema compiles with ajv" {
    run ajv compile -s "$SCHEMA" --spec=draft2020
    [ "$status" -eq 0 ]
    [[ "$output" == *"is valid"* ]]
}

# ── validate-contract.sh: fixture acceptance/rejection ───────────────────────

@test "validate-contract: embedded schema validator has no debug logging" {
    run grep -En 'console\.(log|debug|info|warn)|(^|[^[:alnum:]_])debugger([^[:alnum:]_]|$)' "$SCRIPTS_DIR/validate-contract.sh"
    [ "$status" -eq 1 ]
}

@test "validate-contract: minimal-valid contract exits 0" {
    # Convert YAML fixture to JSON for validate-contract
    run bash -c "yq -o=json '.' '$FIXTURES_DIR/minimal-valid.yml' | '$SCRIPTS_DIR/validate-contract.sh' - '$SCHEMA'"
    [ "$status" -eq 0 ]
}

@test "validate-contract: mode-ii-valid contract exits 0" {
    run bash -c "yq -o=json '.' '$FIXTURES_DIR/mode-ii-valid.yml' | '$SCRIPTS_DIR/validate-contract.sh' - '$SCHEMA'"
    [ "$status" -eq 0 ]
}

@test "validate-contract: empty forbidden_url_patterns without ack exits 2" {
    run bash -c "yq -o=json '.' '$FIXTURES_DIR/empty-forbidden-no-ack.yml' | '$SCRIPTS_DIR/validate-contract.sh' - '$SCHEMA'"
    [ "$status" -eq 2 ]
}

@test "validate-contract: empty forbidden_url_patterns stderr mentions the field" {
    run bash -c "yq -o=json '.' '$FIXTURES_DIR/empty-forbidden-no-ack.yml' | '$SCRIPTS_DIR/validate-contract.sh' - '$SCHEMA' 2>&1"
    [[ "$output" == *"forbidden_url_patterns"* ]]
}

@test "validate-contract: mode-ii-missing-backup exits 2" {
    run bash -c "yq -o=json '.' '$FIXTURES_DIR/mode-ii-missing-backup.yml' | '$SCRIPTS_DIR/validate-contract.sh' - '$SCHEMA'"
    [ "$status" -eq 2 ]
}

@test "validate-contract: mode-ii-missing-backup stderr mentions backup" {
    run bash -c "yq -o=json '.' '$FIXTURES_DIR/mode-ii-missing-backup.yml' | '$SCRIPTS_DIR/validate-contract.sh' - '$SCHEMA' 2>&1"
    [[ "$output" == *"backup"* ]]
}

@test "validate-contract: missing required field mode exits 2" {
    run bash -c "echo '{\"unit\":{}}' | '$SCRIPTS_DIR/validate-contract.sh' - '$SCHEMA'"
    [ "$status" -eq 2 ]
}

@test "validate-contract: invalid mode value exits 2" {
    run bash -c "echo '{\"mode\":\"invalid_mode\"}' | '$SCRIPTS_DIR/validate-contract.sh' - '$SCHEMA'"
    [ "$status" -eq 2 ]
}

@test "validate-contract: missing schema file exits 1" {
    run bash -c "echo '{}' | '$SCRIPTS_DIR/validate-contract.sh' - '/nonexistent/schema.json'"
    [ "$status" -eq 1 ]
}

# ── load-contract.sh: full pipeline ──────────────────────────────────────────

@test "load-contract: minimal-valid repo exits 0 and emits JSON" {
    local fake_repo
    fake_repo=$(make_fake_repo "$FIXTURES_DIR/minimal-valid.yml")
    run "$SCRIPTS_DIR/load-contract.sh" "$fake_repo"
    [ "$status" -eq 0 ]
    # Output should be valid JSON with mode field
    local mode
    mode=$(printf '%s' "$output" | jq -r '.mode')
    [ "$mode" = "strict_isolation" ]
}

@test "load-contract: empty forbidden_url_patterns without ack exits 2" {
    local fake_repo
    fake_repo=$(make_fake_repo "$FIXTURES_DIR/empty-forbidden-no-ack.yml")
    run "$SCRIPTS_DIR/load-contract.sh" "$fake_repo"
    [ "$status" -eq 2 ]
}

@test "load-contract: mode-ii-missing-backup exits 2" {
    local fake_repo
    fake_repo=$(make_fake_repo "$FIXTURES_DIR/mode-ii-missing-backup.yml")
    run "$SCRIPTS_DIR/load-contract.sh" "$fake_repo"
    [ "$status" -eq 2 ]
}

@test "load-contract: mode-ii-valid exits 0" {
    local fake_repo
    fake_repo=$(make_fake_repo "$FIXTURES_DIR/mode-ii-valid.yml")
    run "$SCRIPTS_DIR/load-contract.sh" "$fake_repo"
    [ "$status" -eq 0 ]
}

@test "load-contract: unparseable YAML exits 1" {
    local fake_repo="$TEST_TMPDIR/bad-repo"
    mkdir -p "$fake_repo/.autospec"
    # Write a YAML file that yq cannot parse
    printf 'mode: strict_isolation\n  bad: [unclosed\n' > "$fake_repo/.autospec/test.yml"
    run "$SCRIPTS_DIR/load-contract.sh" "$fake_repo"
    [ "$status" -ne 0 ]
}

@test "load-contract: nonexistent repo_root exits 1" {
    run "$SCRIPTS_DIR/load-contract.sh" "/nonexistent/path/$$"
    [ "$status" -eq 1 ]
}

@test "load-contract: autodetect-only (no test.yml) still validates forbidden_url_patterns fail-closed" {
    # Without a .autospec/test.yml, autodetect fills in empty e2e
    # and validate-contract should refuse-to-run since no forbidden_url_patterns
    local fake_repo="$TEST_TMPDIR/autodetect-repo"
    mkdir -p "$fake_repo"
    run "$SCRIPTS_DIR/load-contract.sh" "$fake_repo"
    # Should refuse to run: no forbidden_url_patterns
    [ "$status" -eq 2 ]
}

@test "load-contract: output JSON has mode field" {
    local fake_repo
    fake_repo=$(make_fake_repo "$FIXTURES_DIR/minimal-valid.yml")
    run "$SCRIPTS_DIR/load-contract.sh" "$fake_repo"
    [ "$status" -eq 0 ]
    run bash -c "printf '%s' '$output' | jq -e '.mode'"
    [ "$status" -eq 0 ]
}

# ── autodetect.sh ─────────────────────────────────────────────────────────────

@test "autodetect: Go repo detects go-cover collector" {
    local fake_repo="$TEST_TMPDIR/go-repo"
    mkdir -p "$fake_repo"
    printf 'module example.com/myapp\n\ngo 1.21\n' > "$fake_repo/go.mod"
    run "$SCRIPTS_DIR/autodetect.sh" "$fake_repo"
    [ "$status" -eq 0 ]
    local collector
    collector=$(printf '%s' "$output" | jq -r '.unit.coverage_collector // empty')
    [ "$collector" = "go-cover" ]
}

@test "autodetect: Python repo detects coverage-py collector" {
    local fake_repo="$TEST_TMPDIR/py-repo"
    mkdir -p "$fake_repo"
    printf '[build-system]\nrequires = ["setuptools"]\n' > "$fake_repo/pyproject.toml"
    run "$SCRIPTS_DIR/autodetect.sh" "$fake_repo"
    [ "$status" -eq 0 ]
    local collector
    collector=$(printf '%s' "$output" | jq -r '.unit.coverage_collector // empty')
    [ "$collector" = "coverage-py" ]
}

@test "autodetect: Rust repo detects cargo-llvm-cov collector" {
    local fake_repo="$TEST_TMPDIR/rust-repo"
    mkdir -p "$fake_repo"
    printf '[package]\nname = "myapp"\nversion = "0.1.0"\n' > "$fake_repo/Cargo.toml"
    run "$SCRIPTS_DIR/autodetect.sh" "$fake_repo"
    [ "$status" -eq 0 ]
    local collector
    collector=$(printf '%s' "$output" | jq -r '.unit.coverage_collector // empty')
    [ "$collector" = "cargo-llvm-cov" ]
}

@test "autodetect: Playwright config detected when file exists" {
    local fake_repo="$TEST_TMPDIR/pw-repo"
    mkdir -p "$fake_repo"
    printf 'export default { use: { baseURL: "http://localhost:3000" } };\n' > "$fake_repo/playwright.config.ts"
    run "$SCRIPTS_DIR/autodetect.sh" "$fake_repo"
    [ "$status" -eq 0 ]
    local cfg
    cfg=$(printf '%s' "$output" | jq -r '.e2e.playwright_config // empty')
    [ "$cfg" = "playwright.config.ts" ]
}

@test "autodetect: nonexistent repo exits 1" {
    run "$SCRIPTS_DIR/autodetect.sh" "/nonexistent/path/$$"
    [ "$status" -eq 1 ]
}

@test "autodetect: empty repo emits valid JSON" {
    local fake_repo="$TEST_TMPDIR/empty-repo"
    mkdir -p "$fake_repo"
    run "$SCRIPTS_DIR/autodetect.sh" "$fake_repo"
    [ "$status" -eq 0 ]
    # Output must be valid JSON
    run bash -c "printf '%s' '$output' | jq . > /dev/null"
    [ "$status" -eq 0 ]
}

# ── schema: acceptance of valid fixtures ──────────────────────────────────────

@test "schema: accepts minimal-valid fixture JSON" {
    local json_file="$TEST_TMPDIR/minimal-valid.json"
    yq -o=json '.' "$FIXTURES_DIR/minimal-valid.yml" > "$json_file"
    run ajv validate -s "$SCHEMA" -d "$json_file" --spec=draft2020
    [ "$status" -eq 0 ]
}

@test "schema: accepts mode-ii-valid fixture JSON" {
    local json_file="$TEST_TMPDIR/mode-ii-valid.json"
    yq -o=json '.' "$FIXTURES_DIR/mode-ii-valid.yml" > "$json_file"
    run ajv validate -s "$SCHEMA" -d "$json_file" --spec=draft2020
    [ "$status" -eq 0 ]
}

@test "schema: rejects object with unknown top-level properties" {
    local json_file="$TEST_TMPDIR/unknown-props.json"
    printf '{"mode":"strict_isolation","unknown_field":"bad","e2e":{"forbidden_url_patterns":["^https?://bad\\.com"]}}\n' > "$json_file"
    run ajv validate -s "$SCHEMA" -d "$json_file" --spec=draft2020
    [ "$status" -ne 0 ]
}

@test "schema: rejects mode=scoped_production without i_understand flag" {
    local json_file="$TEST_TMPDIR/mode-ii-no-ack.json"
    printf '{"mode":"scoped_production","e2e":{"forbidden_url_patterns":["^https?://bad\\.com"]}}\n' > "$json_file"
    run ajv validate -s "$SCHEMA" -d "$json_file" --spec=draft2020
    [ "$status" -ne 0 ]
}

#!/usr/bin/env bats
# skills/autospec-test/tests/unit/wizard.bats
#
# TDD tests for Phase 7: wizard.sh, wizard-probe-backup.sh, wizard-preview.sh
#
# Covers:
#   - Strict-default headless happy path
#   - Mode II headless happy path (with backup driver on PATH)
#   - Mode II refused when no backup driver on PATH (genuine probe invocation)
#   - Wrong ack literal refused (interactive stdin simulation)
#   - Missing --ack-i-understand in headless mode refused
#   - Dry-run preview prints contract without writing files
#   - Ack-lock file created with correct sha format
#   - validate-contract.sh integration (wizard output must pass validation)

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"
    FIXTURES_DIR="$REPO_ROOT/skills/autospec-test/tests/fixtures"
    WIZARD="$SCRIPTS_DIR/wizard.sh"
    PROBE="$SCRIPTS_DIR/wizard-probe-backup.sh"
    PREVIEW="$SCRIPTS_DIR/wizard-preview.sh"
    VALIDATE="$SCRIPTS_DIR/validate-contract.sh"

    TEST_TMPDIR="$(mktemp -d /tmp/autospec-wizard-bats-XXXXXX)"
    AUTOSPEC_DIR="$TEST_TMPDIR/.autospec"
    mkdir -p "$AUTOSPEC_DIR"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# ── Helper: create a fake binary on PATH ─────────────────────────────────────

fake_bin_dir() {
    local dir
    dir="$(mktemp -d /tmp/autospec-fakebin-XXXXXX)"
    echo "$dir"
}

add_fake_binary() {
    local dir="$1"
    local name="$2"
    printf '#!/usr/bin/env bash\necho "fake %s"\n' "$name" > "$dir/$name"
    chmod +x "$dir/$name"
}

# ── Headless strict-default happy path ───────────────────────────────────────

@test "wizard: headless strict-default writes test.yml with mode=strict_isolation" {
    cd "$TEST_TMPDIR"
    run bash "$WIZARD" init \
        --config "$FIXTURES_DIR/wizard/strict-default.yml" \
        --ack-i-understand
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMPDIR/.autospec/test.yml" ]
    grep -q "strict_isolation" "$TEST_TMPDIR/.autospec/test.yml"
}

@test "wizard: headless strict-default does NOT create ack-lock (strict mode)" {
    cd "$TEST_TMPDIR"
    run bash "$WIZARD" init \
        --config "$FIXTURES_DIR/wizard/strict-default.yml" \
        --ack-i-understand
    [ "$status" -eq 0 ]
    # No ack-lock file for strict_isolation mode
    local lock_count
    lock_count=$(find "$TEST_TMPDIR/.autospec" -name '.scoped-prod-acked-*.lock' 2>/dev/null | wc -l | tr -d ' ')
    [ "$lock_count" -eq 0 ]
}

# ── Headless Mode II happy path ───────────────────────────────────────────────

@test "wizard: headless mode-ii happy path writes test.yml and ack-lock" {
    local fake_dir
    fake_dir="$(fake_bin_dir)"
    add_fake_binary "$fake_dir" "pg_dump"

    cd "$TEST_TMPDIR"
    PATH="$fake_dir:$PATH" run bash "$WIZARD" init \
        --config "$FIXTURES_DIR/wizard/mode-ii.yml" \
        --ack-i-understand
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMPDIR/.autospec/test.yml" ]
    grep -q "scoped_production" "$TEST_TMPDIR/.autospec/test.yml"
    # Ack-lock file should exist
    local lock_count
    lock_count=$(find "$TEST_TMPDIR/.autospec" -name '.scoped-prod-acked-*.lock' 2>/dev/null | wc -l | tr -d ' ')
    [ "$lock_count" -eq 1 ]

    rm -rf "$fake_dir"
}

@test "wizard: ack-lock filename matches .scoped-prod-acked-<sha>.lock format" {
    local fake_dir
    fake_dir="$(fake_bin_dir)"
    add_fake_binary "$fake_dir" "pg_dump"

    cd "$TEST_TMPDIR"
    PATH="$fake_dir:$PATH" run bash "$WIZARD" init \
        --config "$FIXTURES_DIR/wizard/mode-ii.yml" \
        --ack-i-understand
    [ "$status" -eq 0 ]
    local lock_file
    lock_file=$(find "$TEST_TMPDIR/.autospec" -name '.scoped-prod-acked-*.lock' | head -1)
    [ -n "$lock_file" ]
    # sha part should be hex chars
    local sha_part
    sha_part=$(basename "$lock_file" | sed 's/^\.scoped-prod-acked-//' | sed 's/\.lock$//')
    [[ "$sha_part" =~ ^[0-9a-f]{8,} ]]

    rm -rf "$fake_dir"
}

# ── validate-contract.sh integration ─────────────────────────────────────────

@test "wizard: output passes validate-contract.sh for strict_isolation mode" {
    cd "$TEST_TMPDIR"
    run bash "$WIZARD" init \
        --config "$FIXTURES_DIR/wizard/strict-default.yml" \
        --ack-i-understand
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMPDIR/.autospec/test.yml" ]
    # Convert wizard YAML output to JSON and validate
    local json_out
    json_out=$(yq -o=json '.' "$TEST_TMPDIR/.autospec/test.yml")
    run bash "$VALIDATE" - <<< "$json_out"
    [ "$status" -eq 0 ]
}

# ── Refused when no backup driver on PATH (Mode II, genuine probe) ─────────────

@test "wizard: refuses Mode II when no backup driver binary on PATH (genuine probe)" {
    # Use mode-ii-no-driver.yml which has no backup.driver field,
    # so wizard falls through to wizard-probe-backup.sh on an empty PATH.
    local empty_dir
    empty_dir="$(fake_bin_dir)"

    cd "$TEST_TMPDIR"
    PATH="$empty_dir" run bash "$WIZARD" init \
        --config "$FIXTURES_DIR/wizard/mode-ii-no-driver.yml" \
        --ack-i-understand
    [ "$status" -ne 0 ]
    [[ "$output" =~ [Bb]ackup|[Dd]river|[Rr]efus ]]
    # No test.yml should be written
    [ ! -f "$TEST_TMPDIR/.autospec/test.yml" ]

    rm -rf "$empty_dir"
}

# ── Wrong ack literal refused (interactive stdin) ─────────────────────────────

@test "wizard: interactive mode refuses wrong-cased ack literal 'i understand'" {
    # Feed "i understand" (wrong casing) on stdin — must exit 1
    cd "$TEST_TMPDIR"
    run bash -c "printf '2\ni understand\n' | bash '$WIZARD' init"
    [ "$status" -ne 0 ]
    [[ "$output" =~ [Rr]efus|[Uu]nderstand ]]
    # No test.yml should be written
    [ ! -f "$TEST_TMPDIR/.autospec/test.yml" ]
}

# ── Headless refused without --ack-i-understand ───────────────────────────────

@test "wizard: headless mode refused without --ack-i-understand flag" {
    cd "$TEST_TMPDIR"
    run bash "$WIZARD" init \
        --config "$FIXTURES_DIR/wizard/strict-default.yml"
    [ "$status" -ne 0 ]
    [[ "$output" =~ [Uu]nderstand|[Aa]ck ]]
    # No test.yml written
    [ ! -f "$TEST_TMPDIR/.autospec/test.yml" ]
}

# ── Dry-run preview ───────────────────────────────────────────────────────────

@test "wizard: --dry-run prints contract preview without writing files" {
    cd "$TEST_TMPDIR"
    run bash "$WIZARD" init \
        --config "$FIXTURES_DIR/wizard/strict-default.yml" \
        --ack-i-understand \
        --dry-run
    [ "$status" -eq 0 ]
    # Output should contain mode
    [[ "$output" =~ mode ]]
    # No test.yml written in dry-run
    [ ! -f "$TEST_TMPDIR/.autospec/test.yml" ]
}

# ── wizard-probe-backup.sh ───────────────────────────────────────────────────

@test "wizard-probe-backup: detects pg_dump when on PATH" {
    local fake_dir
    fake_dir="$(fake_bin_dir)"
    add_fake_binary "$fake_dir" "pg_dump"

    PATH="$fake_dir:$PATH" run bash "$PROBE"
    [ "$status" -eq 0 ]
    [[ "$output" =~ pgdump|pg_dump ]]

    rm -rf "$fake_dir"
}

@test "wizard-probe-backup: exits non-zero when no driver found" {
    local empty_dir
    empty_dir="$(fake_bin_dir)"

    PATH="$empty_dir" run bash "$PROBE"
    [ "$status" -ne 0 ]

    rm -rf "$empty_dir"
}

@test "wizard-probe-backup: detects zfs when on PATH" {
    local fake_dir
    fake_dir="$(fake_bin_dir)"
    add_fake_binary "$fake_dir" "zfs"

    PATH="$fake_dir:$PATH" run bash "$PROBE"
    [ "$status" -eq 0 ]
    [[ "$output" =~ zfs ]]

    rm -rf "$fake_dir"
}

@test "wizard-probe-backup: detects mysqldump when on PATH" {
    local fake_dir
    fake_dir="$(fake_bin_dir)"
    add_fake_binary "$fake_dir" "mysqldump"

    PATH="$fake_dir:$PATH" run bash "$PROBE"
    [ "$status" -eq 0 ]
    [[ "$output" =~ mysqldump ]]

    rm -rf "$fake_dir"
}

# ── wizard-preview.sh ─────────────────────────────────────────────────────────

@test "wizard-preview: prints resolved contract fields to stdout" {
    run bash "$PREVIEW" "$FIXTURES_DIR/wizard/strict-default.yml"
    [ "$status" -eq 0 ]
    [[ "$output" =~ mode ]]
}

@test "wizard-preview: exits non-zero for missing config file" {
    run bash "$PREVIEW" "/nonexistent/config.yml"
    [ "$status" -ne 0 ]
}

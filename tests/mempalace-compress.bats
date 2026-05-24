#!/usr/bin/env bats
# tests/mempalace-compress.bats — tests for skills/autospec-shared/scripts/mempalace-compress.sh
# Covers: below-threshold no-op, above-threshold compress, missing-mempalace silent skip

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/mempalace-compress.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    # Create a small fake memory dir with a few .md files (5 lines total)
    mkdir -p "$TEST_TMP/docs/memory"
    printf 'line1\nline2\nline3\n' > "$TEST_TMP/docs/memory/a.md"
    printf 'line1\nline2\n'       > "$TEST_TMP/docs/memory/b.md"

    # Stub mempalace: records calls to a log file, exits 0
    STUB_MEMPALACE="$TEST_TMP/stub_mempalace"
    CALLS_LOG="$TEST_TMP/mempalace-calls.log"
    cat > "$STUB_MEMPALACE" <<STUB
#!/usr/bin/env bash
echo "mempalace \$*" >> "$CALLS_LOG"
exit 0
STUB
    chmod +x "$STUB_MEMPALACE"

    export TEST_TMP STUB_MEMPALACE CALLS_LOG
}

teardown() {
    rm -rf "$TEST_TMP"
}

# ── basic sanity ──────────────────────────────────────────────────────────────

@test "mempalace-compress.sh exists and is executable" {
    [ -x "$SCRIPT" ]
}

@test "mempalace-compress.sh --help exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
}

# ── below-threshold: no-op ────────────────────────────────────────────────────

@test "below-threshold: exits 0 and does not invoke mempalace" {
    # 5 lines total; threshold 10000 → no-op
    run env MEMPALACE_CMD="$STUB_MEMPALACE" \
        bash "$SCRIPT" --dir "$TEST_TMP/docs/memory" --threshold 10000
    [ "$status" -eq 0 ]
    [ ! -f "$CALLS_LOG" ]
}

@test "below-threshold with --dry-run: exits 0 and does not invoke mempalace" {
    run env MEMPALACE_CMD="$STUB_MEMPALACE" \
        bash "$SCRIPT" --dir "$TEST_TMP/docs/memory" --threshold 10000 --dry-run
    [ "$status" -eq 0 ]
    [ ! -f "$CALLS_LOG" ]
}

# ── above-threshold: triggers compression ────────────────────────────────────

@test "above-threshold: exits 0 and invokes mempalace compress" {
    # 5 lines; threshold 1 → above → compress
    run env MEMPALACE_CMD="$STUB_MEMPALACE" \
        bash "$SCRIPT" --dir "$TEST_TMP/docs/memory" --threshold 1
    [ "$status" -eq 0 ]
    [ -f "$CALLS_LOG" ]
    grep -q "compress" "$CALLS_LOG"
}

@test "above-threshold with --dry-run: passes --dry-run to mempalace" {
    run env MEMPALACE_CMD="$STUB_MEMPALACE" \
        bash "$SCRIPT" --dir "$TEST_TMP/docs/memory" --threshold 1 --dry-run
    [ "$status" -eq 0 ]
    [ -f "$CALLS_LOG" ]
    grep -q "\-\-dry-run" "$CALLS_LOG"
}

@test "above-threshold with --quiet: suppresses stdout" {
    run env MEMPALACE_CMD="$STUB_MEMPALACE" \
        bash "$SCRIPT" --dir "$TEST_TMP/docs/memory" --threshold 1 --quiet
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

# ── missing mempalace: silent skip ────────────────────────────────────────────

@test "missing mempalace: exits 0 silently" {
    # Point MEMPALACE_CMD at a nonexistent path so command -v fails
    run env MEMPALACE_CMD="$TEST_TMP/no_such_mempalace" \
        bash "$SCRIPT" --dir "$TEST_TMP/docs/memory" --threshold 1
    [ "$status" -eq 0 ]
}

@test "missing mempalace: no error output when --quiet" {
    run env MEMPALACE_CMD="$TEST_TMP/no_such_mempalace" \
        bash "$SCRIPT" --dir "$TEST_TMP/docs/memory" --threshold 1 --quiet
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

# ── missing dir: graceful ─────────────────────────────────────────────────────

@test "missing --dir: exits 0 (no-op)" {
    run env MEMPALACE_CMD="$STUB_MEMPALACE" \
        bash "$SCRIPT" --dir "$TEST_TMP/nonexistent" --threshold 100
    [ "$status" -eq 0 ]
}

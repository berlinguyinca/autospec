#!/usr/bin/env bats
# Integration tests for snapshot/custom.sh
# No external deps — uses simple shell commands as the operator cmd.

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/snapshot"

setup() {
    TMP_DIR="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMP_DIR"
}

@test "custom.sh: runs operator command with out_dir as arg" {
    OUT_DIR="$TMP_DIR/out"
    # Command creates a sentinel file in the out_dir
    CMD="touch"
    # touch <out_dir>/sentinel — we use a wrapper script
    WRAPPER="$TMP_DIR/my-snap.sh"
    cat > "$WRAPPER" <<'SCRIPT'
#!/usr/bin/env bash
mkdir -p "$1"
printf 'snapshot-complete\n' > "$1/result.txt"
SCRIPT
    chmod +x "$WRAPPER"
    run bash "$SCRIPT_DIR/custom.sh" "$WRAPPER" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -f "$OUT_DIR/result.txt" ]
}

@test "custom.sh: out_dir is created if it does not exist" {
    OUT_DIR="$TMP_DIR/deep/nested/out"
    WRAPPER="$TMP_DIR/noop.sh"
    cat > "$WRAPPER" <<'SCRIPT'
#!/usr/bin/env bash
mkdir -p "$1"
SCRIPT
    chmod +x "$WRAPPER"
    run bash "$SCRIPT_DIR/custom.sh" "$WRAPPER" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -d "$OUT_DIR" ]
}

@test "custom.sh: propagates non-zero exit from operator command" {
    OUT_DIR="$TMP_DIR/out"
    WRAPPER="$TMP_DIR/fail.sh"
    cat > "$WRAPPER" <<'SCRIPT'
#!/usr/bin/env bash
exit 42
SCRIPT
    chmod +x "$WRAPPER"
    run bash "$SCRIPT_DIR/custom.sh" "$WRAPPER" "$OUT_DIR"
    [ "$status" -eq 42 ]
}

@test "custom.sh: fails with exit 1 if CMD not provided" {
    run bash "$SCRIPT_DIR/custom.sh"
    [ "$status" -eq 1 ]
}

@test "custom.sh: fails with exit 1 if OUT_DIR not provided" {
    run bash "$SCRIPT_DIR/custom.sh" "echo"
    [ "$status" -eq 1 ]
}

@test "custom.sh: fails with exit 2 if command not found" {
    OUT_DIR="$TMP_DIR/out"
    run bash "$SCRIPT_DIR/custom.sh" "/nonexistent/binary" "$OUT_DIR"
    [ "$status" -eq 2 ]
}

@test "custom.sh: passes out_dir as first argument to operator command" {
    OUT_DIR="$TMP_DIR/out"
    WRAPPER="$TMP_DIR/record-arg.sh"
    cat > "$WRAPPER" <<'SCRIPT'
#!/usr/bin/env bash
mkdir -p "$1"
printf '%s\n' "$1" > "$1/received-arg.txt"
SCRIPT
    chmod +x "$WRAPPER"
    run bash "$SCRIPT_DIR/custom.sh" "$WRAPPER" "$OUT_DIR"
    [ "$status" -eq 0 ]
    RECORDED=$(cat "$OUT_DIR/received-arg.txt")
    [ "$RECORDED" = "$OUT_DIR" ]
}

@test "dispatch.sh: routes custom_cmd source correctly" {
    OUT_BASE="$TMP_DIR/clone-snapshots"
    WRAPPER="$TMP_DIR/dispatch-snap.sh"
    cat > "$WRAPPER" <<'SCRIPT'
#!/usr/bin/env bash
mkdir -p "$1"
printf 'custom-done\n' > "$1/custom-result.txt"
SCRIPT
    chmod +x "$WRAPPER"

    CONTRACT_JSON="{\"sources\":[{\"kind\":\"custom_cmd\",\"snapshot_cmd\":\"$WRAPPER\"}],\"expose\":{\"kind\":\"docker_compose\"}}"
    DISPATCH="$SCRIPT_DIR/dispatch.sh"
    run bash -c "printf '%s' '$CONTRACT_JSON' | bash '$DISPATCH' '$OUT_BASE'"
    [ "$status" -eq 0 ]
    RESULT_COUNT=$(find "$OUT_BASE" -name "custom-result.txt" | wc -l | tr -d ' ')
    [ "$RESULT_COUNT" -ge 1 ]
}

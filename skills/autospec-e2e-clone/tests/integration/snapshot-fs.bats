#!/usr/bin/env bats
# Integration tests for snapshot/fs.sh
# Uses temp directories only — no external deps required.

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/snapshot"

setup() {
    TMP_DIR="$(mktemp -d)"
    # Create a fixture directory tree
    SRC_DIR="$TMP_DIR/src"
    mkdir -p "$SRC_DIR/subdir_a" "$SRC_DIR/subdir_b" "$SRC_DIR/subdir_a/nested"

    # Populate files
    for i in $(seq 1 5); do
        printf 'content %d\n' "$i" > "$SRC_DIR/file_${i}.txt"
    done
    for i in $(seq 1 3); do
        printf 'subdir_a content %d\n' "$i" > "$SRC_DIR/subdir_a/a_${i}.txt"
    done
    for i in $(seq 1 4); do
        printf 'subdir_b content %d\n' "$i" > "$SRC_DIR/subdir_b/b_${i}.txt"
    done
    printf 'nested content\n' > "$SRC_DIR/subdir_a/nested/deep.txt"
}

teardown() {
    rm -rf "$TMP_DIR"
}

@test "fs.sh: out_dir is created if it does not exist" {
    OUT_DIR="$TMP_DIR/deep/nested/out"
    run bash "$SCRIPT_DIR/fs.sh" "$SRC_DIR" "50" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -d "$OUT_DIR" ]
}

@test "fs.sh: copies files from source directory" {
    OUT_DIR="$TMP_DIR/out"
    run bash "$SCRIPT_DIR/fs.sh" "$SRC_DIR" "50" "$OUT_DIR"
    [ "$status" -eq 0 ]
    # Some files should be present under out_dir
    FILE_COUNT=$(find "$OUT_DIR" -type f | wc -l | tr -d ' ')
    [ "$FILE_COUNT" -gt 0 ]
}

@test "fs.sh: sampling cap is honored per directory" {
    OUT_DIR="$TMP_DIR/out"
    # Cap at 2 files per directory
    run bash "$SCRIPT_DIR/fs.sh" "$SRC_DIR" "2" "$OUT_DIR"
    [ "$status" -eq 0 ]
    # Root dir has 5 files → capped at 2
    ROOT_OUT="${OUT_DIR}/${SRC_DIR#/}"
    ROOT_COUNT=$(find "$ROOT_OUT" -maxdepth 1 -type f | wc -l | tr -d ' ')
    [ "$ROOT_COUNT" -le 2 ]
}

@test "fs.sh: subdir_b capped at 2 files" {
    OUT_DIR="$TMP_DIR/out"
    run bash "$SCRIPT_DIR/fs.sh" "$SRC_DIR" "2" "$OUT_DIR"
    [ "$status" -eq 0 ]
    SUBDIR_B_OUT="${OUT_DIR}/${SRC_DIR#/}/subdir_b"
    if [ -d "$SUBDIR_B_OUT" ]; then
        B_COUNT=$(find "$SUBDIR_B_OUT" -maxdepth 1 -type f | wc -l | tr -d ' ')
        [ "$B_COUNT" -le 2 ]
    fi
}

@test "fs.sh: nested directories are preserved in output" {
    OUT_DIR="$TMP_DIR/out"
    run bash "$SCRIPT_DIR/fs.sh" "$SRC_DIR" "50" "$OUT_DIR"
    [ "$status" -eq 0 ]
    NESTED_OUT="${OUT_DIR}/${SRC_DIR#/}/subdir_a/nested"
    [ -d "$NESTED_OUT" ]
}

@test "fs.sh: exits 0 with cap=1, only 1 file per dir copied" {
    OUT_DIR="$TMP_DIR/out"
    run bash "$SCRIPT_DIR/fs.sh" "$SRC_DIR" "1" "$OUT_DIR"
    [ "$status" -eq 0 ]
    ROOT_OUT="${OUT_DIR}/${SRC_DIR#/}"
    ROOT_COUNT=$(find "$ROOT_OUT" -maxdepth 1 -type f | wc -l | tr -d ' ')
    [ "$ROOT_COUNT" -le 1 ]
}

@test "fs.sh: fails with exit 1 if no PATHS_CSV given" {
    run bash "$SCRIPT_DIR/fs.sh"
    [ "$status" -eq 1 ]
}

@test "fs.sh: fails with exit 1 if OUT_DIR not given" {
    run bash "$SCRIPT_DIR/fs.sh" "$SRC_DIR"
    [ "$status" -eq 1 ]
}

@test "fs.sh: fails with exit 2 if path does not exist" {
    run bash "$SCRIPT_DIR/fs.sh" "/nonexistent/path" "50" "$TMP_DIR/out"
    [ "$status" -eq 2 ]
}

@test "fs.sh: handles multiple comma-separated paths" {
    OUT_DIR="$TMP_DIR/out"
    DIR_A="$SRC_DIR/subdir_a"
    DIR_B="$SRC_DIR/subdir_b"
    run bash "$SCRIPT_DIR/fs.sh" "${DIR_A},${DIR_B}" "50" "$OUT_DIR"
    [ "$status" -eq 0 ]
    FILE_COUNT=$(find "$OUT_DIR" -type f | wc -l | tr -d ' ')
    [ "$FILE_COUNT" -gt 0 ]
}

@test "dispatch.sh: routes filesystem source correctly" {
    OUT_BASE="$TMP_DIR/clone-snapshots"
    CONTRACT_JSON=$(cat <<JSON
{
  "sources": [
    {
      "kind": "filesystem",
      "paths": ["$SRC_DIR/subdir_a"],
      "sample_files_per_dir": 10
    }
  ],
  "expose": { "kind": "docker_compose" }
}
JSON
)
    DISPATCH="$SCRIPT_DIR/dispatch.sh"
    run bash -c "printf '%s' '$CONTRACT_JSON' | bash '$DISPATCH' '$OUT_BASE'"
    [ "$status" -eq 0 ]
    FILE_COUNT=$(find "$OUT_BASE" -type f | wc -l | tr -d ' ')
    [ "$FILE_COUNT" -gt 0 ]
}

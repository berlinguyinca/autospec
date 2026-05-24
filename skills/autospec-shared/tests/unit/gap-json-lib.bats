#!/usr/bin/env bats
# gap-json-lib.bats — tests for gap-json-lib.sh schema validation + title-hash.

LIB="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/gap-json-lib.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
}
teardown() {
    rm -rf "$TEST_TMP"
}

@test "gap-json-lib.sh --selftest exits 0" {
    run bash "$LIB" --selftest
    [ "$status" -eq 0 ]
}

@test "validate accepts a complete gap object" {
    cat > "$TEST_TMP/gap.json" <<'EOF'
{"gap_id":"G1","dimension":"correctness","severity":"medium","file":"a.sh","line":7,"title":"t","body":"b","dedupe_key":"k1"}
EOF
    run bash "$LIB" --validate-file "$TEST_TMP/gap.json"
    [ "$status" -eq 0 ]
}

@test "validate rejects a gap object missing required keys" {
    cat > "$TEST_TMP/bad.json" <<'EOF'
{"gap_id":"G1","dimension":"correctness"}
EOF
    run bash "$LIB" --validate-file "$TEST_TMP/bad.json"
    [ "$status" -ne 0 ]
}

@test "title-hash is stable and lowercase-hex" {
    run bash "$LIB" --title-hash "cross-repo-search trailing pipe"
    [ "$status" -eq 0 ]
    [[ "$output" =~ ^[0-9a-f]{10}$ ]]
    first="$output"
    run bash "$LIB" --title-hash "cross-repo-search trailing pipe"
    [ "$output" = "$first" ]
}

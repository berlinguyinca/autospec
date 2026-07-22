#!/usr/bin/env bats
# tests/autospec/test_quality_audit_source_scan.bats

setup() {
    AUDIT_SCRIPT="${BATS_TEST_DIRNAME}/../../skills/autospec-shared/scripts/repo-quality-audit.sh"
    TMP_DIR="$(mktemp -d)"
    REPO="$TMP_DIR/repo"
    mkdir -p "$REPO/src" "$REPO/.autospec" "$REPO/dist"
    printf 'console.log("source");\n' > "$REPO/src/main.sh"
    printf 'console.log("ignored");\n' > "$REPO/.autospec/ignored.sh"
    printf 'console.log("generated");\n' > "$REPO/dist/generated.sh"
    JSON_OUT="$TMP_DIR/audit.json"
    MD_OUT="$TMP_DIR/audit.md"
}

teardown() {
    rm -rf "$TMP_DIR"
}

@test "quality audit scans source files but excludes generated and autospec directories" {
    run env \
        AUTOSPEC_QUALITY_AUDIT_LARGE_FILE_LINES=1 \
        AUTOSPEC_QUALITY_AUDIT_DEBUG_THRESHOLD=1 \
        bash "$AUDIT_SCRIPT" --repo "$REPO" --json "$JSON_OUT" --markdown "$MD_OUT"
    [ "$status" -eq 0 ]
    run jq -e '[.findings[] | select(.dedupe_key | startswith("debug-logging-hotspots:src/main.sh|"))] | length == 1' "$JSON_OUT"
    [ "$status" -eq 0 ]
    run jq -e '[.findings[] | select(.file | contains("/.autospec/") or contains("/dist/"))] | length == 0' "$JSON_OUT"
    [ "$status" -eq 0 ]
}

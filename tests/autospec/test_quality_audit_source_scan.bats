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

@test "quality audit does not flag npm scripts when a supported non-npm manifest exists" {
    printf '[package]\nname = "fixture"\nversion = "0.1.0"\n' > "$REPO/Cargo.toml"
    run env bash "$AUDIT_SCRIPT" --repo "$REPO" --json "$JSON_OUT" --markdown "$MD_OUT"
    [ "$status" -eq 0 ]
    run jq -e '[.findings[] | select(.dedupe_key == "package-manifest:missing")] | length == 0' "$JSON_OUT"
    [ "$status" -eq 0 ]
    run jq -e '.verification.lanes.test.status == "not applicable"' "$JSON_OUT"
    [ "$status" -eq 0 ]
}

@test "quality audit reports focused test markers with their source line" {
    printf 'describe.only("focused", () => {});\n' > "$REPO/src/example.spec.ts"
    run env bash "$AUDIT_SCRIPT" --repo "$REPO" --json "$JSON_OUT" --markdown "$MD_OUT"
    [ "$status" -eq 0 ]
    run jq -e '[.findings[] | select(.probe == "focused-skipped-tests" and .file == "src/example.spec.ts")] | length == 1' "$JSON_OUT"
    [ "$status" -eq 0 ]
    run jq -e '.findings[] | select(.probe == "focused-skipped-tests" and .file == "src/example.spec.ts") | .line == 1' "$JSON_OUT"
    [ "$status" -eq 0 ]
}

@test "docs completeness helper has no false-positive any usage finding" {
    local helper="${BATS_TEST_DIRNAME}/../../skills/autospec-run/scripts/docs-completeness-gaps.sh"
    local audit_root="${TMP_DIR}/audit-repo"
    mkdir -p "${audit_root}/skills/autospec-run/scripts"
    cp "$helper" "${audit_root}/skills/autospec-run/scripts/docs-completeness-gaps.sh"
    run env bash "$AUDIT_SCRIPT" --repo "$audit_root" \
        --json "$JSON_OUT" --markdown "$MD_OUT"
    [ "$status" -eq 0 ]
    run jq -e '[.findings[] | select(.probe == "any-usage" and .file == "skills/autospec-run/scripts/docs-completeness-gaps.sh")] | length == 0' "$JSON_OUT"
    [ "$status" -eq 0 ]
}

@test "Playwright author linter has no explicit any type usage" {
    local helper="${BATS_TEST_DIRNAME}/../../skills/autospec-test/scripts/lint-playwright-author.mjs"
    local audit_root="${TMP_DIR}/audit-repo"
    mkdir -p "${audit_root}/skills/autospec-test/scripts"
    cp "$helper" "${audit_root}/skills/autospec-test/scripts/lint-playwright-author.mjs"
    run env bash "$AUDIT_SCRIPT" --repo "$audit_root" \
        --json "$JSON_OUT" --markdown "$MD_OUT"
    [ "$status" -eq 0 ]
    run jq -e '[.findings[] | select(.probe == "any-usage" and .file == "skills/autospec-test/scripts/lint-playwright-author.mjs")] | length == 0' "$JSON_OUT"
    [ "$status" -eq 0 ]
}

@test "Playwright config resolver has no explicit any type usage" {
    local helper="${BATS_TEST_DIRNAME}/../../skills/autospec-test/scripts/playwright-config-resolver.mjs"
    local audit_root="${TMP_DIR}/audit-repo"
    mkdir -p "${audit_root}/skills/autospec-test/scripts"
    cp "$helper" "${audit_root}/skills/autospec-test/scripts/playwright-config-resolver.mjs"
    run env bash "$AUDIT_SCRIPT" --repo "$audit_root" \
        --json "$JSON_OUT" --markdown "$MD_OUT"
    [ "$status" -eq 0 ]
    run jq -e '[.findings[] | select(.probe == "any-usage" and .file == "skills/autospec-test/scripts/playwright-config-resolver.mjs")] | length == 0' "$JSON_OUT"
    [ "$status" -eq 0 ]
}

#!/usr/bin/env bats
# tests/dogfood/test_adapter_layer.bats — issue #646
#
# Covers the adapter layer in scripts/dogfood-detectors.sh:
#   1. Adapter declared in dogfood.yml + present + zero findings → driver PASS.
#   2. Adapter declared but file missing → driver fails with clear error.
#   3. Adapter emits a finding not in allowlist → driver FAIL with offender.
#   4. Allowlist matches the adapter's finding → PASS.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    DRIVER="$REPO_ROOT/scripts/dogfood-detectors.sh"
    TMP="$(mktemp -d -t dogfood-adapter-bats.XXXXXX)"
    mkdir -p "$TMP/scripts" "$TMP/tests/dogfood/allowlist" "$TMP/.autospec"
    cp "$DRIVER" "$TMP/scripts/dogfood-detectors.sh"
    chmod +x "$TMP/scripts/dogfood-detectors.sh"
}

teardown() {
    [ -n "${TMP:-}" ] && [ -d "$TMP" ] && rm -rf "$TMP"
    return 0
}

write_adapter_emits_nothing() {
    cat > "$TMP/scripts/dogfood-adapter-quiet.sh" <<'SH'
#!/usr/bin/env bash
set -eu
# Adapter emits no findings.
exit 0
SH
    chmod +x "$TMP/scripts/dogfood-adapter-quiet.sh"
    printf '[]\n' > "$TMP/tests/dogfood/allowlist/dogfood-adapter-quiet.json"
}

write_adapter_emits_one() {
    cat > "$TMP/scripts/dogfood-adapter-noisy.sh" <<'SH'
#!/usr/bin/env bash
set -eu
printf '{"category":"code_health:doc_drift","rule_id":"DOC_DRIFT_HARD_FAIL","language":"n/a","file":"docs/foo.md","function":"Section","line":0}\n' >> "$VERDICT_FILE"
exit 0
SH
    chmod +x "$TMP/scripts/dogfood-adapter-noisy.sh"
}

write_yml() {
    # write_yml <adapter-basename> [extra-adapter-basename]
    {
        printf 'adapters:\n'
        for name in "$@"; do
            printf '  - detector: scripts/%s-detector.sh\n' "$name"
            printf '    adapter:  scripts/%s.sh\n' "$name"
        done
    } > "$TMP/.autospec/dogfood.yml"
}

@test "adapter present + zero findings → PASS" {
    write_adapter_emits_nothing
    write_yml dogfood-adapter-quiet
    cd "$TMP"
    run bash scripts/dogfood-detectors.sh
    [ "$status" -eq 0 ]
    [[ "$output" == *"dogfood-adapter-quiet.sh findings=0 expected=0 status=PASS"* ]]
}

@test "adapter declared but missing → driver fails with clear error" {
    # No adapter script written; only the yml entry.
    write_yml dogfood-adapter-ghost
    cd "$TMP"
    run bash scripts/dogfood-detectors.sh
    [ "$status" -ne 0 ]
    [[ "$output" == *"dogfood-adapter-ghost.sh"* ]]
    [[ "$output" == *"detector missing or not executable"* ]]
}

@test "adapter emits a finding not in allowlist → FAIL with offender printed" {
    write_adapter_emits_one
    printf '[]\n' > "$TMP/tests/dogfood/allowlist/dogfood-adapter-noisy.json"
    write_yml dogfood-adapter-noisy
    cd "$TMP"
    run bash scripts/dogfood-detectors.sh
    [ "$status" -ne 0 ]
    [[ "$output" == *"dogfood-adapter-noisy.sh findings=1 expected=0 status=FAIL"* ]]
    [[ "$output" == *"docs/foo.md"* ]]
    [[ "$output" == *"DOC_DRIFT_HARD_FAIL"* ]]
}

@test "allowlist matches the adapter's finding → PASS" {
    write_adapter_emits_one
    cat > "$TMP/tests/dogfood/allowlist/dogfood-adapter-noisy.json" <<'JSON'
[
  {"file":"docs/foo.md","function":"Section","rule_id":"DOC_DRIFT_HARD_FAIL","justification":"unit-test fixture"}
]
JSON
    write_yml dogfood-adapter-noisy
    cd "$TMP"
    run bash scripts/dogfood-detectors.sh
    [ "$status" -eq 0 ]
    [[ "$output" == *"dogfood-adapter-noisy.sh findings=1 expected=1 status=PASS"* ]]
}

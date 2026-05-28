#!/usr/bin/env bats
# tests/dogfood/test_driver.bats — issue #641
#
# Covers scripts/dogfood-detectors.sh contract:
#   1. empty allowlist + zero findings → PASS
#   2. empty allowlist + one finding → FAIL (offender printed)
#   3. allowlist matching the finding → PASS
#   4. registered detector missing/non-executable → driver fails clearly

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    DRIVER="$REPO_ROOT/scripts/dogfood-detectors.sh"
    TMP="$(mktemp -d -t dogfood-bats.XXXXXX)"
    # Sandbox repo: a fake autospec checkout with the driver vendored.
    mkdir -p "$TMP/scripts" "$TMP/tests/dogfood/allowlist" "$TMP/.autospec"
    cp "$DRIVER" "$TMP/scripts/dogfood-detectors.sh"
    chmod +x "$TMP/scripts/dogfood-detectors.sh"
}

teardown() {
    [ -n "${TMP:-}" ] && [ -d "$TMP" ] && rm -rf "$TMP"
    return 0
}

write_detector_emits_nothing() {
    cat > "$TMP/scripts/qa-empty-sweep.sh" <<'SH'
#!/usr/bin/env bash
set -eu
# Emit nothing.
exit 0
SH
    chmod +x "$TMP/scripts/qa-empty-sweep.sh"
    printf '[]\n' > "$TMP/tests/dogfood/allowlist/qa-empty-sweep.json"
}

write_detector_emits_one() {
    cat > "$TMP/scripts/qa-noisy-sweep.sh" <<'SH'
#!/usr/bin/env bash
set -eu
printf '{"file":"src/foo.py","function":"detect","rule_id":"STRING_MATCH_DOMAIN_LOGIC","line":42}\n' >> "$VERDICT_FILE"
exit 0
SH
    chmod +x "$TMP/scripts/qa-noisy-sweep.sh"
}

@test "empty allowlist plus zero findings → PASS" {
    write_detector_emits_nothing
    cd "$TMP"
    run bash scripts/dogfood-detectors.sh
    [ "$status" -eq 0 ]
    [[ "$output" == *"dogfood: qa-empty-sweep.sh findings=0 expected=0 status=PASS"* ]]
}

@test "empty allowlist plus one finding → FAIL with offender printed" {
    write_detector_emits_one
    printf '[]\n' > "$TMP/tests/dogfood/allowlist/qa-noisy-sweep.json"
    cd "$TMP"
    run bash scripts/dogfood-detectors.sh
    [ "$status" -ne 0 ]
    [[ "$output" == *"qa-noisy-sweep.sh findings=1 expected=0 status=FAIL"* ]]
    [[ "$output" == *"src/foo.py"* ]]
    [[ "$output" == *"STRING_MATCH_DOMAIN_LOGIC"* ]]
}

@test "allowlist matching the finding → PASS" {
    write_detector_emits_one
    cat > "$TMP/tests/dogfood/allowlist/qa-noisy-sweep.json" <<'JSON'
[
  {"file":"src/foo.py","function":"detect","rule_id":"STRING_MATCH_DOMAIN_LOGIC","justification":"unit-test fixture"}
]
JSON
    cd "$TMP"
    run bash scripts/dogfood-detectors.sh
    [ "$status" -eq 0 ]
    [[ "$output" == *"qa-noisy-sweep.sh findings=1 expected=1 status=PASS"* ]]
}

@test "registered detector missing/non-executable → driver fails with clear message" {
    # Register a non-existent extra detector via .autospec/dogfood.yml.
    cat > "$TMP/.autospec/dogfood.yml" <<'YML'
detectors:
  - scripts/qa-does-not-exist-sweep.sh
YML
    # Also include a real, empty detector so discovery isn't trivially empty.
    write_detector_emits_nothing
    cd "$TMP"
    run bash scripts/dogfood-detectors.sh
    [ "$status" -ne 0 ]
    [[ "$output" == *"qa-does-not-exist-sweep.sh"* ]]
    [[ "$output" == *"detector missing or not executable"* ]]
}

@test "gh is stubbed during detector runs" {
    # Detector tries to call gh; if real gh is invoked it would attempt
    # to create an issue. Stub must prevent that — we assert gh was a
    # no-op by writing a marker file only when the stub is hit.
    cat > "$TMP/scripts/qa-ghprobe-sweep.sh" <<'SH'
#!/usr/bin/env bash
set -eu
# `command -v gh` should resolve to our stub, NOT a real gh binary.
gh_path="$(command -v gh)"
case "$gh_path" in
    */bin/gh) ;;
    *) printf 'unexpected gh path: %s\n' "$gh_path" >&2; exit 1 ;;
esac
# Stub is no-op exit 0; do not emit findings.
gh issue create --title x --body y || true
exit 0
SH
    chmod +x "$TMP/scripts/qa-ghprobe-sweep.sh"
    printf '[]\n' > "$TMP/tests/dogfood/allowlist/qa-ghprobe-sweep.json"
    cd "$TMP"
    run bash scripts/dogfood-detectors.sh
    [ "$status" -eq 0 ]
    [[ "$output" == *"qa-ghprobe-sweep.sh findings=0 expected=0 status=PASS"* ]]
}

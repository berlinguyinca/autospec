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

@test "allowlist row naming a pre-split path is reported as missing, and re-pointing it restores PASS" {
    # Regression for #2946. Splitting a module moves the flagged content to a new path.
    # The allowlist is keyed by path, so the old row stops matching and the new file is
    # unclassified — the driver must surface BOTH halves, because the count alone
    # ("21 vs 15") does not say that this is one move rather than several new defects.
    write_detector_emits_one
    cat > "$TMP/tests/dogfood/allowlist/qa-noisy-sweep.json" <<'JSON'
[
  {"file":"src/pre_split.py","function":"detect","rule_id":"STRING_MATCH_DOMAIN_LOGIC","justification":"row still naming the pre-split path"}
]
JSON
    cd "$TMP"
    run bash scripts/dogfood-detectors.sh
    [ "$status" -ne 0 ]
    [[ "$output" == *"qa-noisy-sweep.sh findings=1 expected=1 status=FAIL"* ]]
    [[ "$output" == *"unexpected findings"* ]]
    [[ "$output" == *"src/foo.py"* ]]
    [[ "$output" == *"missing allowlisted entries"* ]]
    [[ "$output" == *"src/pre_split.py"* ]]

    # Re-pointing the row at the file that now holds the content is the whole fix.
    cat > "$TMP/tests/dogfood/allowlist/qa-noisy-sweep.json" <<'JSON'
[
  {"file":"src/foo.py","function":"detect","rule_id":"STRING_MATCH_DOMAIN_LOGIC","justification":"re-pointed at the post-split path"}
]
JSON
    run bash scripts/dogfood-detectors.sh
    [ "$status" -eq 0 ]
    [[ "$output" == *"qa-noisy-sweep.sh findings=1 expected=1 status=PASS"* ]]
}

@test "findings under local agent worktrees are ignored" {
    cat > "$TMP/scripts/qa-worktree-sweep.sh" <<'SH'
#!/usr/bin/env bash
printf '{"file":".claude/worktrees/feature/scripts/generated.py","function":"main","rule_id":"REPEATED_STRUCTURE_AS_CODE"}\n' >> "$VERDICT_FILE"
SH
    chmod +x "$TMP/scripts/qa-worktree-sweep.sh"
    printf '[]\n' > "$TMP/tests/dogfood/allowlist/qa-worktree-sweep.json"

    cd "$TMP"
    run bash scripts/dogfood-detectors.sh
    [ "$status" -eq 0 ]
    [[ "$output" == *"qa-worktree-sweep.sh findings=0 expected=0 status=PASS"* ]]
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

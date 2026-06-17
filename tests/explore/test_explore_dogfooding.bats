#!/usr/bin/env bats
# tests/explore/test_explore_dogfooding.bats — bats suite for the dogfooding
# researcher (Issue B2).
#
# Asserts:
#   1. Reads a fixture ~/.autospec-shaped dir and emits well-formed proposals.
#   2. Emits empty-output-exit-0 when AUTOSPEC_STATE_DIR points to a nonexistent dir.
#   3. Host-specific absolute paths are redacted from output (no $HOME in output).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t df-test.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"

    # Fixture state dir (shaped like ~/.autospec).
    FAKE_STATE="$TMP/fake-autospec"
    mkdir -p "$FAKE_STATE"
    export AUTOSPEC_STATE_DIR="$FAKE_STATE"
}

teardown() {
    rm -rf "$TMP"
}

assert_well_formed() {
    local json="$1"
    printf '%s' "$json" | python3 -c '
import json, sys
d = json.load(sys.stdin)
assert isinstance(d, dict), "top-level must be object"
assert "source" in d, "missing source"
assert "proposals" in d, "missing proposals"
assert isinstance(d["proposals"], list), "proposals must be list"
for p in d["proposals"]:
    for k in ("title", "evidence", "estimated_complexity", "confidence"):
        assert k in p, f"proposal missing {k}"
    assert p["estimated_complexity"] in ("small", "medium", "large"), "bad complexity"
    assert 0.0 <= float(p["confidence"]) <= 1.0, "bad confidence"
'
}

# ── 1. Well-formed output from fixture state dir ──────────────────────────

@test "dogfooding: emits well-formed JSON when state dir exists but is mostly empty" {
    # State dir exists but has no ledger files — just degrade gracefully.
    git commit -q --allow-empty -m "init"
    run bash "$REPO_ROOT/scripts/explore-research/dogfooding.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"dogfooding"* ]]
}

@test "dogfooding: emits proposals from a seeded failure ledger" {
    # Seed a failure ledger with recurring failures.
    cat > "$FAKE_STATE/failure-ledger.json" <<'EOF'
[
  {"type":"phase4-timeout","message":"Phase 4 timed out"},
  {"type":"phase4-timeout","message":"Phase 4 timed out again"},
  {"type":"phase4-timeout","message":"Phase 4 timed out once more"},
  {"type":"validate-fail","message":"validate.sh returned 1"}
]
EOF
    git commit -q --allow-empty -m "init"
    run bash "$REPO_ROOT/scripts/explore-research/dogfooding.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"dogfooding"* ]]
    # phase4-timeout appears 3 times → should surface a proposal.
    [[ "$output" == *"phase4-timeout"* ]]
}

# ── 2. Empty-output-exit-0 when state dir is absent ──────────────────────

@test "dogfooding: emits empty proposals and exits 0 when state dir is absent" {
    export AUTOSPEC_STATE_DIR="/nonexistent-autospec-state-dir-$$"
    git commit -q --allow-empty -m "init"
    run bash "$REPO_ROOT/scripts/explore-research/dogfooding.sh"
    [ "$status" -eq 0 ]
    # Must emit the empty proposals envelope.
    [[ "$output" == *'"proposals":[]'* || "$output" == *'"proposals": []'* ]]
    [[ "$output" == *"dogfooding"* ]]
}

# ── 3. Host-path redaction ────────────────────────────────────────────────

@test "dogfooding: absolute host paths are redacted from output" {
    # Seed a failure ledger that embeds an absolute path in the message.
    # The SUT must redact it before it reaches any proposal field.
    ABS_PATH="$FAKE_STATE"
    cat > "$FAKE_STATE/failure-ledger.json" <<EOF
[
  {"type":"path-leak","message":"failed to read ${ABS_PATH}/run-summary.md"},
  {"type":"path-leak","message":"failed to read ${ABS_PATH}/run-summary.md again"},
  {"type":"path-leak","message":"failed to read ${ABS_PATH}/run-summary.md thrice"}
]
EOF
    git commit -q --allow-empty -m "init"
    run bash "$REPO_ROOT/scripts/explore-research/dogfooding.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    # The absolute FAKE_STATE path must NOT appear literally in the output.
    # (Redacted to ~/ or ~/.autospec/ form.)
    [[ "$output" != *"$ABS_PATH"* ]]
}

@test "dogfooding: real HOME path is not present in output" {
    # Seed a run-summary that mentions $HOME explicitly.
    # Write to a real temp file to avoid bash 3.2 process-sub issues.
    summary_tmp="$TMP/summary_content.txt"
    printf "# Run summary\n\nResult: PASS\n\n## Next steps\n- check %s/.autospec for issues\n" "$HOME" > "$summary_tmp"
    cp "$summary_tmp" "$FAKE_STATE/run-summary.md"

    git commit -q --allow-empty -m "init"
    run bash "$REPO_ROOT/scripts/explore-research/dogfooding.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    # $HOME must not appear literally in any proposal title or evidence.
    [[ "$output" != *"$HOME/"* ]]
}

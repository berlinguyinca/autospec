#!/usr/bin/env bats
# tests/autospec-run/test_invoke_review_harness_neutral.bats
# Regression for issue #1433: Phase 5.5 invoke-review.sh must be harness-neutral
# and must emit a LOUD diagnostic (never a silent empty gap file) when the
# review backend is unavailable.
#
# AC matrix:
#   AC1: --remediation required    — missing flag → exit 1
#   AC2: backend present (claude)  → dry-run resolves correct argv
#   AC3: backend present (codex)   → dry-run emits codex exec form
#   AC4: backend absent            → exit 0 + code_health: in stderr + non-empty gap file
#   AC5: gap file is NEVER empty when backend absent (no silent skip)

SCRIPT="${BATS_TEST_DIRNAME}/../../skills/autospec-run/scripts/invoke-review.sh"
REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"

setup() {
    TEST_TMP="$(mktemp -d)"
    GAPS_FILE="${TEST_TMP}/gaps.json"
    # Stub PATH: jq available, but no harness binaries unless the test adds them.
    SAFE_PATH="/usr/bin:/bin"
    if command -v jq >/dev/null 2>&1; then
        SAFE_PATH="$(dirname "$(command -v jq)"):${SAFE_PATH}"
    fi
    export TEST_TMP GAPS_FILE SAFE_PATH
}

teardown() {
    rm -rf "${TEST_TMP}"
}

# ── AC1: argument validation ───────────────────────────────────────────────────

@test "AC1: exits 1 when --remediation is missing" {
    run bash "$SCRIPT" --since "2026-01-01T00:00:00Z" --emit-gaps "$GAPS_FILE"
    [ "$status" -eq 1 ]
    [[ "$output" == *"--remediation required"* ]]
}

@test "AC1: exits 1 when --since is missing" {
    run bash "$SCRIPT" --remediation --emit-gaps "$GAPS_FILE"
    [ "$status" -eq 1 ]
    [[ "$output" == *"--since required"* ]]
}

@test "AC1: exits 1 when --emit-gaps is missing" {
    run bash "$SCRIPT" --remediation --since "2026-01-01T00:00:00Z"
    [ "$status" -eq 1 ]
    [[ "$output" == *"--emit-gaps required"* ]]
}

# ── AC2: claude harness dry-run ────────────────────────────────────────────────

@test "AC2: claude harness resolves to 'claude /autospec-review' argv" {
    # Stub a fake claude binary on PATH.
    mkdir -p "${TEST_TMP}/bin"
    printf '#!/usr/bin/env bash\nexit 0\n' > "${TEST_TMP}/bin/claude"
    chmod +x "${TEST_TMP}/bin/claude"

    run env PATH="${TEST_TMP}/bin:${SAFE_PATH}" \
        AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude \
        AUTOSPEC_HANDOFF_DISPATCHER=1 \
        AUTOSPEC_INVOKE_REVIEW_DRY_RUN=1 \
        bash "$SCRIPT" --remediation --since "2026-01-01T00:00:00Z" --emit-gaps "$GAPS_FILE"

    [ "$status" -eq 0 ]
    [[ "$output" == *"DRY-RUN:"* ]]
    [[ "$output" == *"/autospec-review"* ]]
    [[ "$output" == *"--remediation"* ]]
    [[ "$output" == *"--since"* ]]
    [[ "$output" == *"--emit-gaps"* ]]
    # Must NOT use codex exec form
    [[ "$output" != *"exec --skip-git-repo-check"* ]]
}

# ── AC3: codex harness dry-run ────────────────────────────────────────────────

@test "AC3: codex harness resolves to 'codex exec --skip-git-repo-check' form" {
    mkdir -p "${TEST_TMP}/bin"
    printf '#!/usr/bin/env bash\nexit 0\n' > "${TEST_TMP}/bin/codex"
    chmod +x "${TEST_TMP}/bin/codex"

    run env PATH="${TEST_TMP}/bin:${SAFE_PATH}" \
        AUTOSPEC_HANDOFF_DISPATCHER_KIND=codex \
        AUTOSPEC_HANDOFF_DISPATCHER=1 \
        AUTOSPEC_INVOKE_REVIEW_DRY_RUN=1 \
        bash "$SCRIPT" --remediation --since "2026-01-01T00:00:00Z" --emit-gaps "$GAPS_FILE"

    [ "$status" -eq 0 ]
    [[ "$output" == *"exec --skip-git-repo-check"* ]]
    [[ "$output" == *"/autospec-review"* ]]
    [[ "$output" == *"--remediation"* ]]
}

# ── AC4 + AC5: backend absent → LOUD warning + non-empty gap file ─────────────

@test "AC4: exit 0 when backend absent (non-blocking)" {
    run env PATH="${SAFE_PATH}" \
        AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude \
        AUTOSPEC_HARNESS_PROBE_ROOT="${TEST_TMP}" \
        bash "$SCRIPT" --remediation --since "2026-01-01T00:00:00Z" --emit-gaps "$GAPS_FILE"

    [ "$status" -eq 0 ]
}

@test "AC4: code_health:phase55_broad_review_backend_unavailable emitted to stderr" {
    run env PATH="${SAFE_PATH}" \
        AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude \
        AUTOSPEC_HARNESS_PROBE_ROOT="${TEST_TMP}" \
        bash "$SCRIPT" --remediation --since "2026-01-01T00:00:00Z" --emit-gaps "$GAPS_FILE"

    [ "$status" -eq 0 ]
    [[ "$output" == *"code_health:phase55_broad_review_backend_unavailable"* ]]
    [[ "$output" == *"WARNING: Phase 5.5 broad review SKIPPED"* ]]
}

@test "AC5: gap file is non-empty (NOT a silent skip) when backend absent" {
    # Write the gap file path to a real file first so [ -f ] works (bash 3.2 compat).
    touch "$GAPS_FILE"

    env PATH="${SAFE_PATH}" \
        AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude \
        AUTOSPEC_HARNESS_PROBE_ROOT="${TEST_TMP}" \
        bash "$SCRIPT" --remediation --since "2026-01-01T00:00:00Z" --emit-gaps "$GAPS_FILE" \
        >/dev/null 2>&1

    [ -f "$GAPS_FILE" ]
    [ -s "$GAPS_FILE" ]
}

@test "AC5: diagnostic gap carries tooling dimension and non-empty title" {
    touch "$GAPS_FILE"

    env PATH="${SAFE_PATH}" \
        AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude \
        AUTOSPEC_HARNESS_PROBE_ROOT="${TEST_TMP}" \
        bash "$SCRIPT" --remediation --since "2026-01-01T00:00:00Z" --emit-gaps "$GAPS_FILE" \
        >/dev/null 2>&1

    [ -s "$GAPS_FILE" ]
    run jq -e '.[0].dimension == "tooling"' "$GAPS_FILE"
    [ "$status" -eq 0 ]
    run jq -e '.[0].title | length > 0' "$GAPS_FILE"
    [ "$status" -eq 0 ]
    run jq -e '.[0].dedupe_key | startswith("phase55-broad-review-unavailable")' "$GAPS_FILE"
    [ "$status" -eq 0 ]
}

@test "AC5: diagnostic gap appended to existing gap file (not overwritten)" {
    # Pre-populate the gap file with an existing gap.
    printf '[{"gap_id":"G99","dimension":"existing","severity":"high","file":"f","line":1,"title":"existing gap","body":"body","dedupe_key":"existing-1"}]\n' \
        > "$GAPS_FILE"

    env PATH="${SAFE_PATH}" \
        AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude \
        AUTOSPEC_HARNESS_PROBE_ROOT="${TEST_TMP}" \
        bash "$SCRIPT" --remediation --since "2026-01-01T00:00:00Z" --emit-gaps "$GAPS_FILE" \
        >/dev/null 2>&1

    # Both the original gap and the new diagnostic gap must be present.
    run jq 'length' "$GAPS_FILE"
    [ "$status" -eq 0 ]
    [ "$output" -ge 2 ]
    run jq -e 'any(.[]; .dedupe_key == "existing-1")' "$GAPS_FILE"
    [ "$status" -eq 0 ]
    run jq -e 'any(.[]; .dimension == "tooling")' "$GAPS_FILE"
    [ "$status" -eq 0 ]
}

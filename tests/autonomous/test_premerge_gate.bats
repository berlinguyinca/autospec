#!/usr/bin/env bats
# tests/autonomous/test_premerge_gate.bats — unit tests for
# scripts/autonomous-premerge-gate.sh (issue #1376).
#
# Scenarios:
#   1. Clean qa (0 findings) → merge-ok, exit 0.
#   2. High finding fixed within 5 → merge-ok.
#   3. Still-dirty after 5 → blocked + label + notify called.
#   4. Missing configured scan skill → halt code_health, exit 2.
#   5. Low/info finding only → non-blocking, merge-ok.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/autonomous-premerge-gate.sh"

setup() {
    # Isolated temp directory — real home avoided so ~/.autospec/ not polluted.
    TMP="$(mktemp -d -t premerge_gate.XXXXXX)"
    export PATH="$TMP/bin:$PATH"
    mkdir -p "$TMP/bin"

    # Default stubs: autospec-qa skill present (returns 0 findings).
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
# Default stub: no findings.
printf 'autospec-qa: all checks passed\n'
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    # gh stub: accept any args, succeed silently.
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf 'gh stub: %s\n' "$*" >&2
exit 0
EOF
    chmod +x "$TMP/bin/gh"

    # notify.sh stub: record calls for assertion.
    cat > "$TMP/bin/notify.sh" <<'EOF'
#!/usr/bin/env bash
printf 'notify-called: %s | %s\n' "${1:-}" "${2:-}"
exit 0
EOF
    chmod +x "$TMP/bin/notify.sh"

    # Fake git that returns a predictable branch name (macOS bash 3.2 safe).
    cat > "$TMP/bin/git" <<'EOF'
#!/usr/bin/env bash
# Minimal git stub for branch resolution.
case "${1:-}" in
    rev-parse) printf 'feat/test-branch\n'; exit 0 ;;
    *) exit 0 ;;
esac
EOF
    chmod +x "$TMP/bin/git"

    export TMP
}

teardown() {
    [ -n "${TMP:-}" ] && rm -rf "$TMP"
}

# ─── 1. Clean qa → merge-ok ───────────────────────────────────────────────────

@test "clean qa (0 findings) prints merge-ok and exits 0" {
    # Default stub already returns 0 findings.
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

# ─── 2. High finding fixed within 5 → merge-ok ───────────────────────────────

@test "high finding on attempt 1; fixed on attempt 2 → merge-ok" {
    # Write a qa stub that emits a high finding on first call, clean on second.
    local call_file
    call_file="$(mktemp -t qa_calls.XXXXXX)"
    # macOS bash 3.2: write to a real temp file then reference it.
    printf '0\n' > "$call_file"

    cat > "$TMP/bin/autospec-qa" <<EOF
#!/usr/bin/env bash
CALL_FILE="${call_file}"
count=\$(cat "\$CALL_FILE" 2>/dev/null || printf '0')
count=\$((count + 1))
printf '%s\n' "\$count" > "\$CALL_FILE"
if [ "\$count" -le 1 ]; then
    printf 'severity: high — SQL injection in query builder\n'
    exit 1
fi
printf 'autospec-qa: all checks passed\n'
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --max-attempts 5 \
        --notify-sh "$TMP/bin/notify.sh"

    rm -f "$call_file"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

# ─── 3. Still-dirty after 5 → blocked + label + notify ───────────────────────

@test "still-dirty after max attempts → block + label applied + notify called" {
    # qa stub: always emits a high finding.
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
printf 'severity: high — persistent injection vector\n'
exit 1
EOF
    chmod +x "$TMP/bin/autospec-qa"

    # Track gh label calls.
    local gh_calls_file
    gh_calls_file="$(mktemp -t gh_calls.XXXXXX)"
    printf '' > "$gh_calls_file"

    cat > "$TMP/bin/gh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "${gh_calls_file}"
exit 0
EOF
    chmod +x "$TMP/bin/gh"

    # Track notify calls.
    local notify_file
    notify_file="$(mktemp -t notify_calls.XXXXXX)"
    printf '' > "$notify_file"

    cat > "$TMP/bin/notify.sh" <<EOF
#!/usr/bin/env bash
printf 'notify-called: %s\n' "\$*" >> "${notify_file}"
exit 0
EOF
    chmod +x "$TMP/bin/notify.sh"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --repo "testowner/testrepo" \
        --pr 42 \
        --max-attempts 5 \
        --notify-sh "$TMP/bin/notify.sh"

    local gh_log notify_log
    gh_log="$(cat "$gh_calls_file" 2>/dev/null || printf '')"
    notify_log="$(cat "$notify_file" 2>/dev/null || printf '')"
    rm -f "$gh_calls_file" "$notify_file"

    [ "$status" -eq 1 ]
    # Verdict line
    printf '%s\n' "$output" | grep -q "^block"
    # Label applied
    printf '%s\n' "$gh_log" | grep -q "autospec:needs-human"
    # Notify called
    printf '%s\n' "$notify_log" | grep -q "notify-called"
}

# ─── 4. Missing configured scan skill → halt code_health, exit 2 ─────────────

@test "missing autospec-qa skill → halt with code_health identifier, exit 2" {
    # Remove the qa stub from PATH so it's genuinely missing.
    rm -f "$TMP/bin/autospec-qa"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=false

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 2 ]
    printf '%s\n' "$output" | grep -q "^halt code_health:"
}

# ─── 5. Low/info finding only → non-blocking, merge-ok ───────────────────────

@test "low/info findings only → non-blocking, merge-ok" {
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
printf 'severity: low — minor style inconsistency in README\n'
printf 'severity: info — unused import detected\n'
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

# ─── 6. --help exits 0 ────────────────────────────────────────────────────────

@test "--help exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
}

# ─── 7. Retries are bounded: attempt counter matches max-attempts ─────────────

@test "retry loop bounded: max-attempts 3 → exactly 3 qa runs before blocking" {
    local count_file
    count_file="$(mktemp -t qa_count.XXXXXX)"
    printf '0\n' > "$count_file"

    cat > "$TMP/bin/autospec-qa" <<EOF
#!/usr/bin/env bash
COUNT_FILE="${count_file}"
c=\$(cat "\$COUNT_FILE" 2>/dev/null || printf '0')
c=\$((c + 1))
printf '%s\n' "\$c" > "\$COUNT_FILE"
printf 'severity: medium — always-blocking finding\n'
exit 1
EOF
    chmod +x "$TMP/bin/autospec-qa"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --max-attempts 3 \
        --notify-sh "$TMP/bin/notify.sh"

    local final_count
    final_count="$(cat "$count_file" 2>/dev/null || printf '0')"
    rm -f "$count_file"

    [ "$status" -eq 1 ]
    # 3 qa runs made (one per attempt up to max)
    [ "$final_count" -eq 3 ]
}

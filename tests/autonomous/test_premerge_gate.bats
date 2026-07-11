#!/usr/bin/env bats
# tests/autonomous/test_premerge_gate.bats — unit tests for
# scripts/autonomous-premerge-gate.sh (issue #1376 + F4 secaudit #1396).
#
# Scenarios:
#   1. Clean qa + clean secaudit → merge-ok, exit 0.
#   2. High qa finding fixed within 5 → merge-ok.
#   3. Still-dirty after 5 qa attempts → blocked + label + notify called.
#   4. Missing autospec-qa skill → halt code_health:qa_skill_missing, exit 2.
#   5. QA low/info finding only → non-blocking, merge-ok.
#   6. --help exits 0.
#   7. Retry loop bounded: max-attempts 3 → exactly 3 qa runs before blocking.
#   8. Secaudit high finding blocks merge, exit 1.
#   9. Secaudit medium finding blocks merge, exit 1.
#  10. Missing autospec-secaudit skill → halt code_health:secaudit_skill_missing, exit 2.
#  11. Secaudit low/info findings only → non-blocking, merge-ok.
#  12. Secaudit high finding fixed on second attempt → merge-ok.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/autonomous-premerge-gate.sh"

setup() {
    # Isolated temp directory — real home avoided so ~/.autospec/ not polluted.
    TMP="$(mktemp -d -t premerge_gate.XXXXXX)"
    export PATH="$TMP/bin:$PATH"
    export AUTOSPEC_REPO_DIR="$TMP/repo"
    mkdir -p "$TMP/bin" "$AUTOSPEC_REPO_DIR/.autospec"

    # Default stubs: autospec-qa skill present (returns 0 findings).
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
# Default stub: no findings.
printf 'autospec-qa: all checks passed\n'
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    # Default stubs: autospec-secaudit skill present (returns 0 findings).
    cat > "$TMP/bin/autospec-secaudit" <<'EOF'
#!/usr/bin/env bash
# Default stub: no findings.
printf 'autospec-secaudit: all checks passed\n'
exit 0
EOF
    chmod +x "$TMP/bin/autospec-secaudit"

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

# ─── 1. Clean qa + clean secaudit → merge-ok ─────────────────────────────────

@test "clean qa + clean secaudit prints merge-ok and exits 0" {
    # Default stubs already return 0 findings for both qa and secaudit.
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

# ─── 2. High qa finding fixed within 5 → merge-ok ────────────────────────────

@test "high qa finding on attempt 1; fixed on attempt 2 → merge-ok" {
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
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --max-attempts 5 \
        --notify-sh "$TMP/bin/notify.sh"

    rm -f "$call_file"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

# ─── 3. Still-dirty after 5 → blocked + label + notify ───────────────────────

@test "still-dirty after max qa attempts → block + label applied + notify called" {
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
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

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

# ─── 4. Missing autospec-qa skill → halt code_health:qa_skill_missing, exit 2 ─

@test "missing autospec-qa skill → halt with code_health identifier, exit 2" {
    # Remove the qa stub from PATH so it's genuinely missing.
    rm -f "$TMP/bin/autospec-qa"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=false

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 2 ]
    printf '%s\n' "$output" | grep -q "^halt code_health:qa_skill_missing$"
}

# ─── 5. QA low/info finding only → non-blocking, merge-ok ────────────────────

@test "qa low/info findings only → non-blocking, merge-ok" {
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
printf 'severity: low — minor style inconsistency in README\n'
printf 'severity: info — unused import detected\n'
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

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

@test "qa retry loop bounded: max-attempts 3 → exactly 3 qa runs before blocking" {
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
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

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

# ─── 8. Secaudit high finding blocks merge ────────────────────────────────────

@test "secaudit high finding blocks merge, exit 1" {
    # qa is clean; secaudit always emits a high finding.
    cat > "$TMP/bin/autospec-secaudit" <<'EOF'
#!/usr/bin/env bash
printf 'severity: high — hardcoded AWS secret in config.py\n'
exit 1
EOF
    chmod +x "$TMP/bin/autospec-secaudit"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --max-attempts 3 \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "^block"
}

# ─── 9. Secaudit medium finding blocks merge ──────────────────────────────────

@test "secaudit medium finding blocks merge, exit 1" {
    # qa is clean; secaudit always emits a medium finding.
    cat > "$TMP/bin/autospec-secaudit" <<'EOF'
#!/usr/bin/env bash
printf 'severity: medium — SQL injection risk in search handler\n'
exit 1
EOF
    chmod +x "$TMP/bin/autospec-secaudit"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --max-attempts 3 \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "^block"
}

# ─── 10. Missing autospec-secaudit skill → halt fail-closed, exit 2 ──────────

@test "missing autospec-secaudit skill → halt code_health:secaudit_skill_missing, exit 2" {
    # qa present; secaudit absent.
    rm -f "$TMP/bin/autospec-secaudit"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=false

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 2 ]
    printf '%s\n' "$output" | grep -q "^halt code_health:secaudit_skill_missing$"
}

# ─── 11. Secaudit low/info findings only → non-blocking, merge-ok ────────────

@test "secaudit low/info findings only → non-blocking, merge-ok" {
    cat > "$TMP/bin/autospec-secaudit" <<'EOF'
#!/usr/bin/env bash
printf 'severity: low — outdated dependency with no known exploit\n'
printf 'severity: info — license header missing on 2 files\n'
exit 0
EOF
    chmod +x "$TMP/bin/autospec-secaudit"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

# ─── 12. Secaudit high finding fixed on second attempt → merge-ok ────────────

@test "secaudit high finding fixed on attempt 2 → merge-ok" {
    # qa is clean; secaudit emits high on first call, clean on second.
    local secaudit_call_file
    secaudit_call_file="$(mktemp -t secaudit_calls.XXXXXX)"
    printf '0\n' > "$secaudit_call_file"

    cat > "$TMP/bin/autospec-secaudit" <<EOF
#!/usr/bin/env bash
CALL_FILE="${secaudit_call_file}"
count=\$(cat "\$CALL_FILE" 2>/dev/null || printf '0')
count=\$((count + 1))
printf '%s\n' "\$count" > "\$CALL_FILE"
if [ "\$count" -le 1 ]; then
    printf 'severity: high — prompt injection in template renderer\n'
    exit 1
fi
printf 'autospec-secaudit: all checks passed\n'
exit 0
EOF
    chmod +x "$TMP/bin/autospec-secaudit"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --max-attempts 5 \
        --notify-sh "$TMP/bin/notify.sh"

    rm -f "$secaudit_call_file"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

# ─── Native verdict extension (issue #1693 / autotrade #1350) ─────────────────
# The real qa/secaudit skills do not print "severity: high" tokens; qa writes
# .autospec/qa-verdict.json and secaudit prints a "secaudit: must-fix=<N>" line.
# These cases prove the gate honors those native verdicts while the stdout
# severity grep (covered above) stays intact.

# Helper: point the gate at a temp repo dir and seed its .autospec/ dir.
_seed_repo() {
    export AUTOSPEC_REPO_DIR="$TMP/repo"
    mkdir -p "$TMP/repo/.autospec"
}

# ─── 13. qa-verdict.json verdict=FAIL blocks even with empty qa stdout ────────

@test "qa-verdict.json verdict=FAIL blocks merge, exit 1" {
    # qa stdout is silent (no severity tokens); the native verdict must block.
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    _seed_repo
    cat > "$TMP/repo/.autospec/qa-verdict.json" <<'EOF'
{"verdict":"FAIL","findings":[]}
EOF

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --max-attempts 1 \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "^block"
    printf '%s\n' "$output" | grep -qv "^merge-ok$"
}

# ─── 14. qa-verdict.json findings[].release_blocking=true blocks ──────────────

@test "qa-verdict.json release_blocking finding blocks merge, exit 1" {
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    _seed_repo
    cat > "$TMP/repo/.autospec/qa-verdict.json" <<'EOF'
{"verdict":"PARTIAL","findings":[{"id":"F1","release_blocking":false},{"id":"F2","release_blocking":true}]}
EOF

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --max-attempts 1 \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "^block"
}

# ─── 15. qa-verdict.json verdict=PASS → merge-ok ─────────────────────────────

@test "qa-verdict.json verdict=PASS + clean secaudit → merge-ok" {
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    _seed_repo
    cat > "$TMP/repo/.autospec/qa-verdict.json" <<'EOF'
{"verdict":"PASS","findings":[{"id":"F1","release_blocking":false}]}
EOF

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

# ─── 16. qa-verdict.json verdict=PARTIAL (no release_blocking) → merge-ok ─────

@test "qa-verdict.json verdict=PARTIAL without release_blocking → merge-ok" {
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    _seed_repo
    cat > "$TMP/repo/.autospec/qa-verdict.json" <<'EOF'
{"verdict":"PARTIAL","findings":[{"id":"F1","release_blocking":false}]}
EOF

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

# ─── 17. secaudit "must-fix=2" summary line blocks ───────────────────────────

@test "secaudit must-fix=2 summary line blocks merge, exit 1" {
    # secaudit stdout has no severity tokens, only the deterministic summary.
    cat > "$TMP/bin/autospec-secaudit" <<'EOF'
#!/usr/bin/env bash
printf 'secaudit: must-fix=2 advisory=0 scanners-degraded=none\n'
exit 0
EOF
    chmod +x "$TMP/bin/autospec-secaudit"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --max-attempts 1 \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "^block"
}

# ─── 18. secaudit "must-fix=0" summary line → merge-ok ───────────────────────

@test "secaudit must-fix=0 summary line → merge-ok" {
    cat > "$TMP/bin/autospec-secaudit" <<'EOF'
#!/usr/bin/env bash
printf 'secaudit: must-fix=0 advisory=3 scanners-degraded=none\n'
exit 0
EOF
    chmod +x "$TMP/bin/autospec-secaudit"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

# ─── 19. Malformed qa-verdict.json does not crash → stdout-only behavior ──────

@test "malformed qa-verdict.json falls back to stdout-only, does not crash" {
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    _seed_repo
    # Not valid JSON.
    printf '{ this is not json ::: \n' > "$TMP/repo/.autospec/qa-verdict.json"

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --notify-sh "$TMP/bin/notify.sh"

    # No blocking severity tokens on stdout and malformed verdict → merge-ok.
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
    # WARN surfaced to stderr (bats merges into $output).
    printf '%s\n' "$output" | grep -q "WARN: could not parse"
}

# ─── 20. Native PASS verdict is authoritative over the skill's own [medium] ───
#         advisory stdout lines (autospec #1718 / autotrade #1350 AC #3). The qa
#         skill emits "[medium]" process advisories (dirty-git-status,
#         missing-manifest) it itself marks non-blocking; the legacy stdout grep
#         used to block them, overriding the native PASS. It must not.

@test "qa-verdict.json verdict=PASS + [medium] advisory stdout → merge-ok" {
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
printf -- '- [medium] dirty-git-status / current-branch-regression — working tree has uncommitted changes\n'
printf -- '- [medium] package-manager-scripts / autospec-process-gap — missing package manager manifest\n'
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    _seed_repo
    cat > "$TMP/repo/.autospec/qa-verdict.json" <<'EOF'
{"verdict":"PASS","findings":[{"id":"F1","release_blocking":false}]}
EOF

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

# ─── 21. Backstop: even with a native PASS verdict, a high/severe/critical ────
#         stdout token still blocks (guards against a verdict that under-reports
#         a real severe finding). Only [medium] is demoted, not high/critical.

@test "qa-verdict.json verdict=PASS but critical stdout token still blocks" {
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
printf 'severity: critical — hardcoded private key committed to source\n'
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    _seed_repo
    cat > "$TMP/repo/.autospec/qa-verdict.json" <<'EOF'
{"verdict":"PASS","findings":[{"id":"F1","release_blocking":false}]}
EOF

    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    # Single attempt so the backstop-block is observed without a fix loop.
    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --max-attempts 1 \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -qv "^merge-ok$"
}

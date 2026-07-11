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
    mkdir -p "$TMP/bin"

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
    if [ -n "${EXTRA_WT:-}" ]; then rm -rf "$EXTRA_WT"; fi
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


# ─── 22. Worktree-local qa verdict beats stale parent AUTOSPEC_REPO_DIR ──────

@test "qa-verdict lookup uses active /tmp/wt-* cwd, not stale parent AUTOSPEC_REPO_DIR" {
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"

    local parent_repo active_wt
    parent_repo="$TMP/parent"
    active_wt="$(mktemp -d /tmp/wt-premerge-gate.XXXXXX)"
    export EXTRA_WT="$active_wt"
    mkdir -p "$parent_repo/.autospec" "$active_wt/.autospec"
    mkdir -p "$parent_repo/.git"
    cat > "$parent_repo/.autospec/qa-verdict.json" <<'EOF'
{"verdict":"FAIL","findings":[{"id":"STALE","release_blocking":true}]}
EOF
    cat > "$active_wt/.autospec/qa-verdict.json" <<'EOF'
{"verdict":"PASS","findings":[]}
EOF

    export AUTOSPEC_REPO_DIR="$parent_repo"
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash -c "cd '$active_wt' && bash '$SCRIPT' --pr-branch 'feat/test-branch' --notify-sh '$TMP/bin/notify.sh'"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

# ─── 22-26. self-originated gate (issue #1742) ────────────────────────────────
# New scripts/autonomous-guardrails.sh self-originated subcommand, wired into
# autonomous-premerge-gate.sh mirroring the diff-guard block
# (autonomous-premerge-gate.sh:~338-371): block self_originated_direct_merge +
# autospec:needs-human label + exit 1. Per
# docs/specs/2026-07-10-autonomous-integration-branch-design.md §Error handling
# fenced-surfaces ordering, blast-radius quarantine evaluates FIRST and wins.

# Stub gh for the self-originated gate's lookups:
#   gh pr view <pr> --repo <repo> --json baseRefName        -> $base
#   gh repo view <repo> --json defaultBranchRef              -> $default_branch
#   gh pr view <pr> --repo <repo> --json closingIssuesReferences -> $issue
#   gh api repos/<repo>/issues/<issue>[/comments|/timeline]   -> provenance lookups
_stub_gh_self_originated() {
    local base="$1" default_branch="$2" issue="$3" origin_self="$4" user_type="${5:-User}" user_login="${6:-a-human}"
    local labels_json='[]'
    if [ "$origin_self" = "1" ]; then
        labels_json='[{"name":"origin:self"}]'
    fi
    cat > "$TMP/bin/gh" <<GHSTUB
#!/usr/bin/env bash
case "\$*" in
    *"pr view"*"baseRefName"*)
        printf '%s\n' "$base" ;;
    *"repo view"*"defaultBranchRef"*)
        printf '%s\n' "$default_branch" ;;
    *"pr view"*"closingIssuesReferences"*)
        printf '%s\n' "$issue" ;;
    *"api"*"issues/$issue"*"comments"*)
        printf '[]\n' ;;
    *"api"*"issues/$issue"*"timeline"*)
        printf '[]\n' ;;
    *"api"*"issues/$issue"*)
        printf '{"labels":%s,"user":{"login":"%s","type":"%s"}}\n' '$labels_json' "$user_login" "$user_type" ;;
    *"pr edit"*)
        printf 'labeled\n' ;;
    *)
        exit 0 ;;
esac
GHSTUB
    chmod +x "$TMP/bin/gh"
}

@test "self-originated PR on protected parent (main) blocks with needs-human label" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"
    _stub_gh_self_originated "main" "main" "777" "1"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 501 --repo acme/widgets \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q "block self_originated_direct_merge"
}

@test "self-originated PR based on the integration branch passes the check" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"
    _stub_gh_self_originated "autospec/autonomous-main" "main" "777" "1"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 502 --repo acme/widgets \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

@test "operator-originated PR based on the parent passes the check" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"
    _stub_gh_self_originated "main" "main" "778" "0" "User" "alice"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 503 --repo acme/widgets \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

@test "blast-radius quarantine on a fenced surface wins over the self-originated gate" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"
    _stub_gh_self_originated "main" "main" "779" "1"

    printf 'scripts/autonomous-example.sh\n' > "$TMP/changed-files.txt"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 504 --repo acme/widgets \
        --check-self-originated \
        --changed-files "$TMP/changed-files.txt" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "quarantine fenced_surface"
    printf '%s\n' "$output" | grep -qv "self_originated_direct_merge"
}

@test "self-originated PR with no linked issue fails closed and blocks" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"
    _stub_gh_self_originated "main" "main" "" "1"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 505 --repo acme/widgets \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q "block self_originated_direct_merge"
}

@test "unresolved PR base ref fails closed and blocks" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"
    # Empty base simulates a gh/API failure resolving baseRefName.
    _stub_gh_self_originated "" "main" "780" "1"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 506 --repo acme/widgets \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q "block self_originated_direct_merge"
}

@test "unresolved default branch treats base as protected; self provenance blocks" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"
    # Empty default branch simulates a gh repo view failure; base main must
    # still be treated as protected (fail closed), so self provenance blocks.
    _stub_gh_self_originated "main" "" "781" "1"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 507 --repo acme/widgets \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q "block self_originated_direct_merge"
}

# ─── 29-36. quarantine-review regression tests (PR #1757 findings) ────────────

@test "provenance resolver exit-0 printing 'unknown' fails closed and blocks" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"
    _stub_gh_self_originated "main" "main" "782" "0" "User" "alice"

    # Resolver test seam: exit 0 but output is not exactly `operator`.
    cat > "$TMP/bin/provenance-stub.sh" <<'STUB'
#!/usr/bin/env bash
printf 'unknown\n'
exit 0
STUB
    chmod +x "$TMP/bin/provenance-stub.sh"
    export AUTOSPEC_PROVENANCE_SH="$TMP/bin/provenance-stub.sh"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 508 --repo acme/widgets \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q "block self_originated_direct_merge"
}

@test "provenance resolver partial stdout then exit 1 fails closed and blocks" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"
    _stub_gh_self_originated "main" "main" "783" "0" "User" "alice"

    # Partial output ('oper', no newline) then crash: the old
    # `|| printf 'self'` idiom concatenated this into 'operself' and any
    # non-'self' string allowed. Must block.
    cat > "$TMP/bin/provenance-stub.sh" <<'STUB'
#!/usr/bin/env bash
printf 'oper'
exit 1
STUB
    chmod +x "$TMP/bin/provenance-stub.sh"
    export AUTOSPEC_PROVENANCE_SH="$TMP/bin/provenance-stub.sh"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 509 --repo acme/widgets \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q "block self_originated_direct_merge"
}

@test "empty integration_branch_prefix in base config does not exempt main" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"
    _stub_gh_self_originated "main" "main" "784" "1"

    # Base config sets an empty prefix — without the guard, ""* matches
    # EVERY base and the whole gate is exempted.
    cat > "$TMP/bin/git" <<'STUB'
#!/usr/bin/env bash
case "${1:-}" in
    rev-parse) printf 'feat/test-branch\n'; exit 0 ;;
    show) printf 'autonomous:\n  self_originated:\n    integration_branch_prefix: ""\n'; exit 0 ;;
    *) exit 0 ;;
esac
STUB
    chmod +x "$TMP/bin/git"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 510 --repo acme/widgets \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q "block self_originated_direct_merge"
}

@test "worktree config allow_direct_merge=true is ignored; base config decides" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    _stub_gh_self_originated "main" "main" "785" "1"

    # The PR's own (worktree) config tries to disarm the gate. The gate must
    # read policy from the merge base (git show), which yields no such key,
    # so the built-in false applies and the PR blocks.
    cat > "$TMP/worktree-autospec.yml" <<'CFG'
autonomous:
  self_originated:
    allow_direct_merge: true
CFG
    export AUTOSPEC_CONFIG_FILE="$TMP/worktree-autospec.yml"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 511 --repo acme/widgets \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q "block self_originated_direct_merge"
}

@test "allow_direct_merge=true in BASE config allows the self PR" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"
    _stub_gh_self_originated "main" "main" "786" "1"

    cat > "$TMP/bin/git" <<'STUB'
#!/usr/bin/env bash
case "${1:-}" in
    rev-parse) printf 'feat/test-branch\n'; exit 0 ;;
    show) printf 'autonomous:\n  self_originated:\n    allow_direct_merge: true\n'; exit 0 ;;
    *) exit 0 ;;
esac
STUB
    chmod +x "$TMP/bin/git"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 512 --repo acme/widgets \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^merge-ok$"
}

@test "two linked issues, first operator second self, blocks" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"

    # A spurious extra 'Closes #<operator-issue>' listed FIRST must not
    # launder the self-originated issue #901 past the gate.
    cat > "$TMP/bin/gh" <<'GHSTUB'
#!/usr/bin/env bash
case "$*" in
    *"pr view"*"baseRefName"*) printf 'main\n' ;;
    *"repo view"*"defaultBranchRef"*) printf 'main\n' ;;
    *"pr view"*"closingIssuesReferences"*) printf '900\n901\n' ;;
    *"issues/900/comments"*) printf '[]\n' ;;
    *"issues/900/timeline"*) printf '[]\n' ;;
    *"issues/900"*) printf '{"labels":[],"user":{"login":"alice","type":"User"}}\n' ;;
    *"issues/901/comments"*) printf '[]\n' ;;
    *"issues/901/timeline"*) printf '[]\n' ;;
    *"issues/901"*) printf '{"labels":[{"name":"origin:self"}],"user":{"login":"autospec-bot","type":"Bot"}}\n' ;;
    *"pr edit"*) printf 'labeled\n' ;;
    *) exit 0 ;;
esac
GHSTUB
    chmod +x "$TMP/bin/gh"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 513 --repo acme/widgets \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q "block self_originated_direct_merge"
    printf '%s\n' "$output" | grep -q "ISSUE:901"
}

@test "protected-branches CSV entries are whitespace-trimmed" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true
    export AUTOSPEC_CONFIG_FILE="$TMP/missing-autospec.yml"
    # Base 'release' only matches via the ' release' CSV entry after trim.
    _stub_gh_self_originated "release" "main" "787" "1"

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --pr 514 --repo acme/widgets \
        --check-self-originated \
        --protected-branches "main, release" \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q "block self_originated_direct_merge"
}

@test "--check-self-originated without --pr/--repo dies with exit 3" {
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$SCRIPT" \
        --pr-branch "feat/test-branch" \
        --check-self-originated \
        --notify-sh "$TMP/bin/notify.sh"

    [ "$status" -eq 3 ]
}

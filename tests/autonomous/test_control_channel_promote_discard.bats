#!/usr/bin/env bats
# tests/autonomous/test_control_channel_promote_discard.bats — conductor
# consumer for the control-channel `promote`/`discard` reserved labels
# (docs/specs/2026-07-10-autonomous-integration-branch-design.md,
# §Architecture item 8).
#
# Covers:
#   1. Trusted promote: merges the CI-green roll-up PR (no --delete-branch)
#      then resets — order pinned via a shared sequence log — closes the
#      control issue, clears the trigger label, clears the #1767
#      self-originated pause marker.
#   2. Refused promote/discard (control channel vetoed): comment only, no
#      action, trigger label cleared (no per-cycle re-emit).
#   3. Promote no-op paths (no roll-up) comment + clear the label.
#   4. Red/unsettled CI: promote refuses to merge, comments, clears label.
#   5. Merge-ok/reset-fail: control issue stays open; a re-fired promote
#      with the roll-up MERGED re-attempts reset (idempotent recovery).
#   6. Discard: reopen list comes from the PR BODY manifest (never from
#      spoofable PR comments), close verified before anything else happens,
#      re-run posts no duplicate reopen comments, pause marker cleared.
#
# Mocking strategy mirrors tests/autonomous/test_conductor_provenance_dispatch.bats:
# helper scripts stubbed via CONDUCTOR_SCRIPTS_DIR; gh stubbed via a fake PATH
# dir that logs every invocation; bash 3.2-safe (no process substitution;
# fixtures written to real temp files). No real GitHub calls.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"

    TEST_TMP="$(mktemp -d)"
    export HOME="$TEST_TMP"
    mkdir -p "$HOME/.autospec"

    FAKE_SCRIPTS="$TEST_TMP/fake-scripts"
    mkdir -p "$FAKE_SCRIPTS"

    FAKE_BIN="$TEST_TMP/fake-bin"
    mkdir -p "$FAKE_BIN"
    export PATH="$FAKE_BIN:$PATH"

    GH_LOG="$TEST_TMP/gh-calls.log"
    INTBRANCH_LOG="$TEST_TMP/intbranch-calls.log"
    SEQ_LOG="$TEST_TMP/seq.log"
    WRITEBACK_LOG="$TEST_TMP/writeback-calls.log"
    touch "$GH_LOG" "$INTBRANCH_LOG" "$SEQ_LOG" "$WRITEBACK_LOG"

    # repo_root inside the conductor = parent of CONDUCTOR_SCRIPTS_DIR.
    mkdir -p "$TEST_TMP/.autospec"
    PAUSE_FILE="$TEST_TMP/.autospec/self-originated-pause.json"

    # No board configured unless a test calls _write_board_cache /
    # _install_board_writeback: AUTOSPEC_STATE_DIR defaults to
    # $HOME/.autospec (== TEST_TMP/.autospec here), so with no board-cache
    # dir created, _autospec_conductor_board_state's very first check
    # returns immediately — this is the default, common-case posture for
    # every pre-existing test in this file.
    unset AUTOSPEC_BOARD_WRITEBACK_SCRIPT

    export LOOP_LIB REPO_ROOT FAKE_SCRIPTS TEST_TMP FAKE_BIN \
        GH_LOG INTBRANCH_LOG SEQ_LOG PAUSE_FILE WRITEBACK_LOG
}

teardown() {
    rm -rf "$TEST_TMP" 2>/dev/null || true
}

_install_stub() {
    local name="$1" body="$2"
    printf '#!/usr/bin/env bash\n%s\n' "$body" > "$FAKE_SCRIPTS/$name"
    chmod +x "$FAKE_SCRIPTS/$name"
}

_install_common_stubs() {
    _install_stub "autonomous-premerge-gate.sh" 'printf "merge-ok\n"'
    _install_stub "autonomous-spend-ledger.sh" \
        'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
    _install_stub "autonomous-resilience.sh" \
        'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
    _install_stub "autospec-usage-limit.sh" 'exit 0'
    _install_stub "autonomous-waterfall.sh" \
        'printf '\''{"tier":1,"action":"run-backlog","reason":"test"}\n'\'''
}

# Fake gh — logs every invocation to GH_LOG and SEQ_LOG, routes queries to
# per-test fixture files:
#   $TEST_TMP/ci.json                 statusCheckRollup array (default [])
#   $TEST_TMP/pr-body.txt             roll-up PR body (default empty)
#   $TEST_TMP/pr-close-rc             exit code for `pr close` (default 0)
#   $TEST_TMP/issue-reopen-rc         exit code for `issue reopen` (default 0)
#   $TEST_TMP/issue-reopen-rc-<n>     per-issue reopen exit code override
#   $TEST_TMP/issue-comments-<n>.txt  bodies for `issue view <n> --json comments`
# `repo view` answers the defaultBranchRef jq output ("main").
_install_gh() {
    cat > "$FAKE_BIN/gh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$GH_LOG"
printf 'gh %s\n' "\$*" >> "$SEQ_LOG"
case "\${1:-} \${2:-}" in
    "repo view")
        echo "main"
        ;;
    "pr view")
        case "\$*" in
            *statusCheckRollup*) cat "$TEST_TMP/ci.json" 2>/dev/null || echo "[]" ;;
            *"--json body"*)     cat "$TEST_TMP/pr-body.txt" 2>/dev/null || true ;;
            *)                   echo "" ;;
        esac
        ;;
    "pr close")
        rc="\$(cat "$TEST_TMP/pr-close-rc" 2>/dev/null || printf '0')"
        exit "\$rc"
        ;;
    "issue view")
        cat "$TEST_TMP/issue-comments-\${3:-}.txt" 2>/dev/null || true
        ;;
    "issue reopen")
        rc="\$(cat "$TEST_TMP/issue-reopen-rc-\${3:-}" 2>/dev/null || cat "$TEST_TMP/issue-reopen-rc" 2>/dev/null || printf '0')"
        exit "\$rc"
        ;;
    "issue list")
        echo "[]"
        ;;
    *)
        exit 0
        ;;
esac
EOF
    chmod +x "$FAKE_BIN/gh"
}

# control-channel stub: emits a fixed DECISION sequence for one cycle.
_install_control_channel() {
    local decision_lines="$1"
    cat > "$FAKE_SCRIPTS/autonomous-control-channel.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "$decision_lines"
EOF
    chmod +x "$FAKE_SCRIPTS/autonomous-control-channel.sh"
}

# integration-branch stub: `status` emits fixed rollup_pr number/state (pass
# rollup_pr as a bare number or "null"; rollup_state as a bare word like OPEN,
# or "null" — this helper does the JSON quoting itself). `reset` logs + exits
# with the rc recorded in $TEST_TMP/reset-rc (default 0). Both are logged to
# SEQ_LOG for cross-stub ordering assertions.
_install_intbranch() {
    local rollup_pr="${1:-null}" rollup_state_raw="${2:-null}" rollup_state_json
    if [ "$rollup_state_raw" = "null" ]; then
        rollup_state_json="null"
    else
        rollup_state_json="\"$rollup_state_raw\""
    fi
    cat > "$FAKE_SCRIPTS/autonomous-integration-branch.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$INTBRANCH_LOG"
printf 'intbranch %s\n' "\$*" >> "$SEQ_LOG"
case "\${1:-}" in
    status)
        printf '{"branch":"autospec/autonomous-main","rollup_pr":{"number":$rollup_pr,"state":$rollup_state_json},"accumulated_pr_count":1,"age_days":1,"diff_lines":10}\n'
        ;;
    reset)
        rc="\$(cat "$TEST_TMP/reset-rc" 2>/dev/null || printf '0')"
        exit "\$rc"
        ;;
    *) exit 0 ;;
esac
EOF
    chmod +x "$FAKE_SCRIPTS/autonomous-integration-branch.sh"
}

# Board-cache fixture consumed by _autospec_conductor_board_state: a plan
# JSON mapping (repo, issue number) -> item_id, dropped straight into
# $HOME/.autospec/board-cache (HOME == TEST_TMP per setup()) — the exact
# location _autospec_conductor_board_state scans.
_write_board_cache() {
    local repo="$1" issue="$2" item_id="$3"
    mkdir -p "$TEST_TMP/.autospec/board-cache"
    cat > "$TEST_TMP/.autospec/board-cache/plan.json" <<EOF
{"items":[{"repo":"$repo","number":$issue,"item_id":"$item_id"}]}
EOF
}

# project-board-writeback.sh stub: logs every invocation ("--item X --state
# Y") to WRITEBACK_LOG, exits with the rc recorded in
# $TEST_TMP/writeback-rc (default 0, mirroring the real script's
# always-exits-0 contract unless a test deliberately overrides it to prove
# the failure is swallowed).
_install_board_writeback() {
    cat > "$FAKE_SCRIPTS/project-board-writeback.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$WRITEBACK_LOG"
rc="\$(cat "$TEST_TMP/writeback-rc" 2>/dev/null || printf '0')"
exit "\$rc"
EOF
    chmod +x "$FAKE_SCRIPTS/project-board-writeback.sh"
    export AUTOSPEC_BOARD_WRITEBACK_SCRIPT="$FAKE_SCRIPTS/project-board-writeback.sh"
}

_run_cycle() {
    run bash -c "
        . '$LOOP_LIB'
        CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
        CONDUCTOR_REPO='test-owner/test-repo' \
        CONDUCTOR_MAX_CYCLES=1 \
        CONDUCTOR_POLL_INTERVAL=0 \
        CONDUCTOR_DRY_RUN=0 \
        CONDUCTOR_NO_DIGEST=1 \
        AUTOSPEC_RUN_CMD= \
        AUTOSPEC_INTEGRATION_BRANCH_BIN='$FAKE_SCRIPTS/autonomous-integration-branch.sh' \
        autospec_conductor_run
    " 2>&1
}

# Same cycle, but with CONDUCTOR_DRY_RUN=1 — used only by the board
# write-back dry-run proof; the promote/merge control decision path itself
# is not gated by CONDUCTOR_DRY_RUN (a pre-existing, unrelated fact), so
# this still merges — it only proves _autospec_conductor_board_state's own
# dry-run guard.
_run_cycle_dry() {
    run bash -c "
        . '$LOOP_LIB'
        CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
        CONDUCTOR_REPO='test-owner/test-repo' \
        CONDUCTOR_MAX_CYCLES=1 \
        CONDUCTOR_POLL_INTERVAL=0 \
        CONDUCTOR_DRY_RUN=1 \
        CONDUCTOR_NO_DIGEST=1 \
        AUTOSPEC_RUN_CMD= \
        AUTOSPEC_INTEGRATION_BRANCH_BIN='$FAKE_SCRIPTS/autonomous-integration-branch.sh' \
        autospec_conductor_run
    " 2>&1
}

# Fixture: a roll-up PR body whose manifest lists issues 11 and 12.
_write_manifest_body() {
    cat > "$TEST_TMP/pr-body.txt" <<'EOF'
Roll-up PR intro text.
<!-- autospec-rollup-manifest:begin -->
## Autonomous roll-up manifest

- **Integration branch:** `autospec/autonomous-main` -> `main`
- **Accumulated merged PRs:** 2
- **Landed issues:**
  - #11 — first feature (worker PR [#101](url), +10/-2) — origin: origin:self
  - #12 — second feature
<!-- autospec-rollup-manifest:end -->
Trailing text mentioning - #999 outside the manifest markers.
EOF
}

# ---------------------------------------------------------------------------
# promote: trusted actor (control-channel already vetted) — merges + resets.
# ---------------------------------------------------------------------------

@test "promote merges the CI-green roll-up (no --delete-branch) then resets, closes issue, clears label + pause marker" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:501"
    _install_gh
    _install_intbranch 42 "OPEN"
    printf '[]' > "$TEST_TMP/ci.json"
    printf '{"reason":"rollup-red"}\n' > "$PAUSE_FILE"

    _run_cycle
    [ "$status" -eq 0 ]

    # Merge is admin+squash but NEVER --delete-branch (reset recreates the branch).
    grep -q "^pr merge 42 --repo test-owner/test-repo --admin --squash$" "$GH_LOG"
    ! grep -q -- "--delete-branch" "$GH_LOG"
    grep -q "^reset --parent main --repo test-owner/test-repo$" "$INTBRANCH_LOG"
    # Order pinned via the shared sequence log: merge BEFORE reset.
    merge_line="$(grep -n "^gh pr merge 42" "$SEQ_LOG" | head -1 | cut -d: -f1)"
    reset_line="$(grep -n "^intbranch reset" "$SEQ_LOG" | head -1 | cut -d: -f1)"
    [ -n "$merge_line" ]
    [ -n "$reset_line" ]
    [ "$merge_line" -lt "$reset_line" ]
    grep -q "^issue close 501 --repo test-owner/test-repo$" "$GH_LOG"
    # Trigger label cleared (one-shot: no per-cycle re-emit).
    grep -q "^issue edit 501 --repo test-owner/test-repo --remove-label autospec:promote$" "$GH_LOG"
    # #1767 pause marker cleared: promote is the operator exit from rollup-red.
    [ ! -f "$PAUSE_FILE" ]
}

@test "refused promote: comment only, no merge call, trigger label cleared" {
    _install_common_stubs
    _install_control_channel "DECISION:promote-refused
PROMOTE_ISSUE:502"
    _install_gh
    _install_intbranch 42 "OPEN"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr merge" "$GH_LOG"
    [ ! -s "$INTBRANCH_LOG" ]
    grep -q "^issue comment 502 --repo test-owner/test-repo" "$GH_LOG"
    grep -q "^issue edit 502 --repo test-owner/test-repo --remove-label autospec:promote$" "$GH_LOG"
}

@test "refused discard: comment only, no close call, trigger label cleared" {
    _install_common_stubs
    _install_control_channel "DECISION:discard-refused
DISCARD_ISSUE:504"
    _install_gh
    _install_intbranch 42 "OPEN"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr close" "$GH_LOG"
    ! grep -q "^issue reopen" "$GH_LOG"
    grep -q "^issue comment 504 --repo test-owner/test-repo" "$GH_LOG"
    grep -q "^issue edit 504 --repo test-owner/test-repo --remove-label autospec:discard$" "$GH_LOG"
}

@test "promote with no open roll-up is a clean no-op (comment, no merge, no reset, label cleared)" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:503"
    _install_gh
    _install_intbranch "null" "null"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr merge" "$GH_LOG"
    ! grep -q "^reset" "$INTBRANCH_LOG"
    grep -q "^issue comment 503 --repo test-owner/test-repo" "$GH_LOG"
    grep -q "^issue edit 503 --repo test-owner/test-repo --remove-label autospec:promote$" "$GH_LOG"
}

@test "promote with red CI is refused: comment, no merge call, label cleared" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:505"
    _install_gh
    _install_intbranch 42 "OPEN"
    printf '[{"name":"pytest","conclusion":"FAILURE"}]' > "$TEST_TMP/ci.json"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr merge" "$GH_LOG"
    ! grep -q "^reset" "$INTBRANCH_LOG"
    grep -q "^issue comment 505 --repo test-owner/test-repo" "$GH_LOG"
    grep -q "^issue edit 505 --repo test-owner/test-repo --remove-label autospec:promote$" "$GH_LOG"
}

@test "promote with unsettled (pending) CI is refused: no merge call" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:506"
    _install_gh
    _install_intbranch 42 "OPEN"
    printf '[{"name":"pytest","conclusion":null}]' > "$TEST_TMP/ci.json"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr merge" "$GH_LOG"
    grep -q "^issue comment 506 --repo test-owner/test-repo" "$GH_LOG"
}

@test "promote with STARTUP_FAILURE conclusion is refused (allowlist, not blocklist)" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:508"
    _install_gh
    _install_intbranch 42 "OPEN"
    printf '[{"name":"pytest","conclusion":"STARTUP_FAILURE"}]' > "$TEST_TMP/ci.json"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr merge" "$GH_LOG"
    grep -q "^issue comment 508 --repo test-owner/test-repo" "$GH_LOG"
}

@test "promote with all-green conclusions (SUCCESS/NEUTRAL/SKIPPED) merges" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:509"
    _install_gh
    _install_intbranch 42 "OPEN"
    printf '[{"name":"a","conclusion":"SUCCESS"},{"name":"b","conclusion":"NEUTRAL"},{"name":"c","conclusion":"SKIPPED"}]' > "$TEST_TMP/ci.json"

    _run_cycle
    [ "$status" -eq 0 ]

    grep -q "^pr merge 42 --repo test-owner/test-repo --admin --squash$" "$GH_LOG"
}

@test "merge-ok/reset-fail leaves the control issue open; re-fired promote on MERGED roll-up recovers via reset" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:507"
    _install_gh
    _install_intbranch 42 "OPEN"
    printf '[]' > "$TEST_TMP/ci.json"
    printf '1' > "$TEST_TMP/reset-rc"

    _run_cycle
    [ "$status" -eq 0 ]

    # Phase 1: merged, reset failed → NO issue close (stays open for retry),
    # label still cleared (operator re-fires by re-applying it).
    grep -q "^pr merge 42 --repo test-owner/test-repo --admin --squash$" "$GH_LOG"
    ! grep -q "^issue close 507" "$GH_LOG"
    grep -q "^issue edit 507 --repo test-owner/test-repo --remove-label autospec:promote$" "$GH_LOG"

    # Phase 2: operator re-applies the label; the roll-up is now MERGED and
    # reset succeeds → recovery path resets WITHOUT re-merging, closes the
    # issue, clears the pause marker.
    : > "$GH_LOG"; : > "$INTBRANCH_LOG"; : > "$SEQ_LOG"
    _install_intbranch 42 "MERGED"
    printf '0' > "$TEST_TMP/reset-rc"
    printf '{"reason":"rollup-red"}\n' > "$PAUSE_FILE"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr merge" "$GH_LOG"
    grep -q "^reset --parent main --repo test-owner/test-repo$" "$INTBRANCH_LOG"
    grep -q "^issue close 507 --repo test-owner/test-repo$" "$GH_LOG"
    grep -q "^issue edit 507 --repo test-owner/test-repo --remove-label autospec:promote$" "$GH_LOG"
    [ ! -f "$PAUSE_FILE" ]
}

# ---------------------------------------------------------------------------
# discard: manifest-driven reopen, verified close, idempotent re-run.
# ---------------------------------------------------------------------------

@test "discard closes the roll-up, reopens manifest issues with comments, clears label + pause marker" {
    _install_common_stubs
    _install_control_channel "DECISION:discard
DISCARD_ISSUE:601"
    _install_gh
    _install_intbranch 77 "OPEN"
    _write_manifest_body
    printf '{"reason":"rollup-red"}\n' > "$PAUSE_FILE"

    _run_cycle
    [ "$status" -eq 0 ]

    grep -q "^pr close 77 --repo test-owner/test-repo --delete-branch$" "$GH_LOG"
    grep -q "^issue reopen 11 --repo test-owner/test-repo$" "$GH_LOG"
    grep -q "^issue reopen 12 --repo test-owner/test-repo$" "$GH_LOG"
    grep -q "^issue comment 11 --repo test-owner/test-repo --body discarded-from-rollup" "$GH_LOG"
    grep -q "^issue comment 12 --repo test-owner/test-repo --body discarded-from-rollup" "$GH_LOG"
    grep -q "^issue close 601 --repo test-owner/test-repo$" "$GH_LOG"
    grep -q "^issue edit 601 --repo test-owner/test-repo --remove-label autospec:discard$" "$GH_LOG"
    [ ! -f "$PAUSE_FILE" ]
    # Refs outside the manifest markers are never honored.
    ! grep -q "^issue reopen 999" "$GH_LOG"
}

@test "discard reopen list ignores fake markers in PR comments (body manifest is authoritative)" {
    _install_common_stubs
    _install_control_channel "DECISION:discard
DISCARD_ISSUE:604"
    _install_gh
    _install_intbranch 77 "OPEN"
    # Body manifest lists ONLY #11. A stranger's PR comment with fake
    # rollup markers for #999 must be ignored — the handler never reads
    # PR comments for the reopen list.
    cat > "$TEST_TMP/pr-body.txt" <<'EOF'
<!-- autospec-rollup-manifest:begin -->
- **Landed issues:**
  - #11 — the only real landed issue
<!-- autospec-rollup-manifest:end -->
EOF

    _run_cycle
    [ "$status" -eq 0 ]

    grep -q "^issue reopen 11 --repo test-owner/test-repo$" "$GH_LOG"
    ! grep -q "^issue reopen 999" "$GH_LOG"
    ! grep -q "^issue comment 999" "$GH_LOG"
    # The reopen list must not have been derived from PR comments at all.
    ! grep -q "^pr view 77 --repo test-owner/test-repo --json comments" "$GH_LOG"
}

@test "discard aborts when pr close fails: nothing reopened, control issue left open, label cleared" {
    _install_common_stubs
    _install_control_channel "DECISION:discard
DISCARD_ISSUE:605"
    _install_gh
    _install_intbranch 77 "OPEN"
    _write_manifest_body
    printf '1' > "$TEST_TMP/pr-close-rc"

    _run_cycle
    [ "$status" -eq 0 ]

    grep -q "^pr close 77" "$GH_LOG"
    ! grep -q "^issue reopen" "$GH_LOG"
    ! grep -q "^issue close 605" "$GH_LOG"
    grep -q "^issue comment 605 --repo test-owner/test-repo" "$GH_LOG"
    grep -q "^issue edit 605 --repo test-owner/test-repo --remove-label autospec:discard$" "$GH_LOG"
}

@test "discard re-run posts no duplicate reopen comments (idempotency marker in issue comments)" {
    _install_common_stubs
    _install_control_channel "DECISION:discard
DISCARD_ISSUE:606"
    _install_gh
    _install_intbranch 77 "OPEN"
    _write_manifest_body
    # Issue 11 already carries the discarded-from-rollup comment for PR #77
    # (prior run before a crash); issue 12 does not.
    printf 'discarded-from-rollup: roll-up PR #77 was discarded via control-channel issue #606; reopened for re-drain.\n' \
        > "$TEST_TMP/issue-comments-11.txt"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^issue reopen 11 " "$GH_LOG"
    ! grep -q "^issue comment 11 --repo test-owner/test-repo --body discarded-from-rollup" "$GH_LOG"
    grep -q "^issue reopen 12 --repo test-owner/test-repo$" "$GH_LOG"
    grep -q "^issue comment 12 --repo test-owner/test-repo --body discarded-from-rollup" "$GH_LOG"
}

@test "discard with reopen failures: partial-failure comment, control issue left open, pause marker kept" {
    _install_common_stubs
    _install_control_channel "DECISION:discard
DISCARD_ISSUE:607"
    _install_gh
    _install_intbranch 77 "OPEN"
    _write_manifest_body
    printf '1' > "$TEST_TMP/issue-reopen-rc"
    printf '{"reason":"rollup-red"}\n' > "$PAUSE_FILE"

    _run_cycle
    [ "$status" -eq 0 ]

    grep -q "^pr close 77" "$GH_LOG"
    grep -q "^issue reopen 11" "$GH_LOG"
    # Reopen failed → no per-issue comment, no control-issue close, no
    # success claim; label still cleared so the operator re-fires explicitly.
    ! grep -q "^issue comment 11 --repo test-owner/test-repo --body discarded-from-rollup" "$GH_LOG"
    ! grep -q "^issue close 607" "$GH_LOG"
    grep -q "^issue comment 607 --repo test-owner/test-repo" "$GH_LOG"
    grep -q "^issue edit 607 --repo test-owner/test-repo --remove-label autospec:discard$" "$GH_LOG"
    # Pause marker must NOT be cleared on a partial failure.
    [ -f "$PAUSE_FILE" ]
}

@test "discard retry after close-ok/reopen-fail reopens remainder and closes control issue" {
    _install_common_stubs
    _install_control_channel "DECISION:discard
DISCARD_ISSUE:608"
    _install_gh
    _install_intbranch 77 "OPEN"
    _write_manifest_body
    printf '1' > "$TEST_TMP/issue-reopen-rc-12"
    printf '{"reason":"rollup-red"}\n' > "$PAUSE_FILE"

    _run_cycle
    [ "$status" -eq 0 ]

    grep -q "^pr close 77 --repo test-owner/test-repo --delete-branch$" "$GH_LOG"
    grep -q "^issue reopen 11 --repo test-owner/test-repo$" "$GH_LOG"
    grep -q "^issue comment 11 --repo test-owner/test-repo --body discarded-from-rollup" "$GH_LOG"
    grep -q "^issue reopen 12 --repo test-owner/test-repo$" "$GH_LOG"
    ! grep -q "^issue comment 12 --repo test-owner/test-repo --body discarded-from-rollup" "$GH_LOG"
    ! grep -q "^issue close 608" "$GH_LOG"
    [ -f "$TEST_TMP/.autospec/discard-pending.json" ]
    [ -f "$PAUSE_FILE" ]

    : > "$GH_LOG"; : > "$INTBRANCH_LOG"; : > "$SEQ_LOG"
    _install_intbranch 77 "CLOSED"
    rm -f "$TEST_TMP/issue-reopen-rc-12"
    printf 'discarded-from-rollup: roll-up PR #77 was discarded via control-channel issue #608; reopened for re-drain.\n' \
        > "$TEST_TMP/issue-comments-11.txt"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr close 77" "$GH_LOG"
    ! grep -q "^issue reopen 11 " "$GH_LOG"
    ! grep -q "^issue comment 11 --repo test-owner/test-repo --body discarded-from-rollup" "$GH_LOG"
    grep -q "^issue reopen 12 --repo test-owner/test-repo$" "$GH_LOG"
    grep -q "^issue comment 12 --repo test-owner/test-repo --body discarded-from-rollup" "$GH_LOG"
    grep -q "^issue close 608 --repo test-owner/test-repo$" "$GH_LOG"
    grep -q "^issue edit 608 --repo test-owner/test-repo --remove-label autospec:discard$" "$GH_LOG"
    [ ! -f "$TEST_TMP/.autospec/discard-pending.json" ]
    [ ! -f "$PAUSE_FILE" ]
}

@test "discard with a merged roll-up is a clean no-op (never reopens landed issues)" {
    _install_common_stubs
    _install_control_channel "DECISION:discard
DISCARD_ISSUE:603"
    _install_gh
    _install_intbranch 88 "MERGED"
    _write_manifest_body

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr close" "$GH_LOG"
    ! grep -q "^issue reopen" "$GH_LOG"
    grep -q "^issue comment 603 --repo test-owner/test-repo" "$GH_LOG"
    grep -q "^issue edit 603 --repo test-owner/test-repo --remove-label autospec:discard$" "$GH_LOG"
}

@test "discard with no open roll-up is a clean no-op (comment only, label cleared)" {
    _install_common_stubs
    _install_control_channel "DECISION:discard
DISCARD_ISSUE:602"
    _install_gh
    _install_intbranch "null" "null"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr close" "$GH_LOG"
    ! grep -q "^issue reopen" "$GH_LOG"
    grep -q "^issue comment 602 --repo test-owner/test-repo" "$GH_LOG"
    grep -q "^issue edit 602 --repo test-owner/test-repo --remove-label autospec:discard$" "$GH_LOG"
}

# ---------------------------------------------------------------------------
# Board write-back (project-board-fleet-execution Plan B Task 5): promote
# fires Testing (checks polled) then Done (merged + issue closed) via
# _autospec_conductor_board_state — decorative only, must never affect the
# merge path.
# ---------------------------------------------------------------------------

@test "promote fires board Testing then Done, in order, with the resolved item id" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:701"
    _install_gh
    _install_intbranch 42 "OPEN"
    printf '[{"name":"a","conclusion":"SUCCESS"}]' > "$TEST_TMP/ci.json"
    _install_board_writeback
    _write_board_cache "test-owner/test-repo" 701 "PVTI_xyz"

    _run_cycle
    [ "$status" -eq 0 ]

    # Merge still happened — board write-back is decorative, not a gate.
    grep -q "^pr merge 42 --repo test-owner/test-repo --admin --squash$" "$GH_LOG"
    grep -q "^issue close 701 --repo test-owner/test-repo$" "$GH_LOG"

    grep -q -- "--item PVTI_xyz --state Testing" "$WRITEBACK_LOG"
    grep -q -- "--item PVTI_xyz --state Done" "$WRITEBACK_LOG"
    testing_line="$(grep -n -- "--state Testing" "$WRITEBACK_LOG" | head -1 | cut -d: -f1)"
    done_line="$(grep -n -- "--state Done" "$WRITEBACK_LOG" | head -1 | cut -d: -f1)"
    [ -n "$testing_line" ]
    [ -n "$done_line" ]
    [ "$testing_line" -lt "$done_line" ]
}

@test "a failing board write-back does not fail promote or change its exit status" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:702"
    _install_gh
    _install_intbranch 43 "OPEN"
    printf '[{"name":"a","conclusion":"SUCCESS"}]' > "$TEST_TMP/ci.json"
    _install_board_writeback
    _write_board_cache "test-owner/test-repo" 702 "PVTI_fails"
    printf '1\n' > "$TEST_TMP/writeback-rc"

    _run_cycle
    [ "$status" -eq 0 ]

    # The write-back was attempted (and failed) but the merge path is
    # completely unaffected: merge + reset + close all still happened.
    grep -q -- "--item PVTI_fails --state Testing" "$WRITEBACK_LOG"
    grep -q "^pr merge 43 --repo test-owner/test-repo --admin --squash$" "$GH_LOG"
    grep -q "^reset --parent main --repo test-owner/test-repo$" "$INTBRANCH_LOG"
    grep -q "^issue close 702 --repo test-owner/test-repo$" "$GH_LOG"
}

@test "no board cache configured: promote merges cleanly with zero write-back calls" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:703"
    _install_gh
    _install_intbranch 44 "OPEN"
    printf '[{"name":"a","conclusion":"SUCCESS"}]' > "$TEST_TMP/ci.json"
    _install_board_writeback
    # Deliberately no _write_board_cache call: no board-cache dir exists.

    _run_cycle
    [ "$status" -eq 0 ]

    grep -q "^pr merge 44 --repo test-owner/test-repo --admin --squash$" "$GH_LOG"
    run cat "$WRITEBACK_LOG"
    [ -z "$output" ]
}

@test "board cache present but no matching item: promote merges cleanly with zero write-back calls" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:704"
    _install_gh
    _install_intbranch 45 "OPEN"
    printf '[{"name":"a","conclusion":"SUCCESS"}]' > "$TEST_TMP/ci.json"
    _install_board_writeback
    # Cache exists, but for a different repo/issue entirely.
    _write_board_cache "some-other/repo" 999 "PVTI_unrelated"

    _run_cycle
    [ "$status" -eq 0 ]

    grep -q "^pr merge 45 --repo test-owner/test-repo --admin --squash$" "$GH_LOG"
    run cat "$WRITEBACK_LOG"
    [ -z "$output" ]
}

@test "CONDUCTOR_DRY_RUN=1: zero board write-back calls even though a matching item exists" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:705"
    _install_gh
    _install_intbranch 46 "OPEN"
    printf '[{"name":"a","conclusion":"SUCCESS"}]' > "$TEST_TMP/ci.json"
    _install_board_writeback
    _write_board_cache "test-owner/test-repo" 705 "PVTI_dryrun"

    _run_cycle_dry
    [ "$status" -eq 0 ]

    run cat "$WRITEBACK_LOG"
    [ -z "$output" ]
}

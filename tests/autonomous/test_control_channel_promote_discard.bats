#!/usr/bin/env bats
# tests/autonomous/test_control_channel_promote_discard.bats — conductor
# consumer for the control-channel `promote`/`discard` reserved labels
# (docs/specs/2026-07-10-autonomous-integration-branch-design.md,
# §Architecture item 8).
#
# Covers:
#   1. Trusted-actor promote: merges the roll-up PR and resets the
#      integration branch.
#   2. Untrusted-actor promote is refused upstream by
#      autonomous-control-channel.sh (DECISION:promote-refused) — the
#      conductor consumer must never call `gh pr merge`.
#   3. Discard: closes the roll-up PR (which deletes the integration
#      branch), reopens its manifest issues with a discarded-from-rollup
#      comment.
#   4. Promote with no open roll-up: clean no-op with a comment, no merge
#      call and no reset call.
#
# Mocking strategy mirrors tests/autonomous/test_conductor_provenance_dispatch.bats:
# helper scripts stubbed via CONDUCTOR_SCRIPTS_DIR; gh stubbed via a fake PATH
# dir that logs every invocation to a file; bash 3.2-safe (no process
# substitution; fixtures written to real temp files). No real GitHub calls.

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
    touch "$GH_LOG"
    export GH_LOG

    INTBRANCH_LOG="$TEST_TMP/intbranch-calls.log"
    touch "$INTBRANCH_LOG"

    export LOOP_LIB REPO_ROOT FAKE_SCRIPTS TEST_TMP FAKE_BIN GH_LOG INTBRANCH_LOG
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

# Fake gh — logs every invocation as a single space-joined line, and answers
# `pr view <N> --json comments` fixed fixture / `repo` lookups fail-open.
_install_gh() {
    local comments_json="${1:-[]}"
    cat > "$FAKE_BIN/gh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$GH_LOG"
case "\${1:-}" in
    issue)
        case "\${2:-}" in
            list) echo "[]" ;;
            *) exit 0 ;;
        esac
        ;;
    pr)
        case "\${2:-}" in
            view) printf '%s' '$comments_json' | jq -c '{comments: .}' ;;
            *) exit 0 ;;
        esac
        ;;
    repo) echo '{"nameWithOwner":"test-owner/test-repo"}' ;;
    *) exit 0 ;;
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
# or "null" for "no roll-up" — this helper does the JSON quoting itself so
# callers never hand-quote JSON literals). `reset` logs + exits 0 unless
# $TEST_TMP/reset-rc says otherwise.
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

_run_cycle() {
    run bash -c "
        . '$LOOP_LIB'
        CONDUCTOR_SCRIPTS_DIR='$FAKE_SCRIPTS' \
        CONDUCTOR_REPO='test-owner/test-repo' \
        CONDUCTOR_MAX_CYCLES=1 \
        CONDUCTOR_POLL_INTERVAL=0 \
        CONDUCTOR_DRY_RUN=0 \
        CONDUCTOR_NO_DIGEST=1 \
        AUTOSPEC_INTEGRATION_BRANCH_BIN='$FAKE_SCRIPTS/autonomous-integration-branch.sh' \
        autospec_conductor_run
    " 2>&1
}

# ---------------------------------------------------------------------------
# promote: trusted actor (control-channel already vetted) — merges + resets.
# ---------------------------------------------------------------------------

@test "promote by trusted actor merges the roll-up PR and resets the integration branch" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:501"
    _install_gh '[]'
    _install_intbranch 42 "OPEN"

    _run_cycle
    [ "$status" -eq 0 ]

    grep -q "^pr merge 42 --repo test-owner/test-repo --admin --squash --delete-branch$" "$GH_LOG"
    grep -q "^reset --parent main --repo test-owner/test-repo$" "$INTBRANCH_LOG"
    grep -q "^issue close 501 --repo test-owner/test-repo$" "$GH_LOG"
}

# ---------------------------------------------------------------------------
# promote: untrusted actor — control-channel refuses; conductor must never
# call `gh pr merge`.
# ---------------------------------------------------------------------------

@test "promote by untrusted actor is refused: no merge call, comment only" {
    _install_common_stubs
    _install_control_channel "DECISION:promote-refused
PROMOTE_ISSUE:502"
    _install_gh '[]'
    _install_intbranch 42 "OPEN"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr merge" "$GH_LOG"
    [ ! -s "$INTBRANCH_LOG" ]
    grep -q "^issue comment 502 --repo test-owner/test-repo" "$GH_LOG"
}

# ---------------------------------------------------------------------------
# promote: no open roll-up — clean no-op, comment only, no merge/reset.
# ---------------------------------------------------------------------------

@test "promote with no open roll-up is a clean no-op (comment, no merge, no reset)" {
    _install_common_stubs
    _install_control_channel "DECISION:promote
PROMOTE_ISSUE:503"
    _install_gh '[]'
    _install_intbranch "null" "null"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr merge" "$GH_LOG"
    ! grep -q "^reset" "$INTBRANCH_LOG"
    grep -q "^issue comment 503 --repo test-owner/test-repo" "$GH_LOG"
}

# ---------------------------------------------------------------------------
# discard: closes the roll-up PR (deletes the branch via --delete-branch),
# reopens its manifest issues with a discarded-from-rollup comment.
# ---------------------------------------------------------------------------

@test "discard closes the roll-up PR, deletes the branch, and reopens its issues" {
    _install_common_stubs
    _install_control_channel "DECISION:discard
DISCARD_ISSUE:601"
    _install_gh '[{"body":"<!-- autospec-rollup:issue-11 -->\nlanded #11"},{"body":"<!-- autospec-rollup:issue-12 -->\nlanded #12"}]'
    _install_intbranch 77 "OPEN"

    _run_cycle
    [ "$status" -eq 0 ]

    grep -q "^pr close 77 --repo test-owner/test-repo --delete-branch$" "$GH_LOG"
    grep -q "^issue reopen 11 --repo test-owner/test-repo$" "$GH_LOG"
    grep -q "^issue reopen 12 --repo test-owner/test-repo$" "$GH_LOG"
    grep -q "^issue comment 11 --repo test-owner/test-repo --body discarded-from-rollup" "$GH_LOG"
    grep -q "^issue comment 12 --repo test-owner/test-repo --body discarded-from-rollup" "$GH_LOG"
    grep -q "^issue close 601 --repo test-owner/test-repo$" "$GH_LOG"
}

@test "discard with no open roll-up is a clean no-op (comment only)" {
    _install_common_stubs
    _install_control_channel "DECISION:discard
DISCARD_ISSUE:602"
    _install_gh '[]'
    _install_intbranch "null" "null"

    _run_cycle
    [ "$status" -eq 0 ]

    ! grep -q "^pr close" "$GH_LOG"
    ! grep -q "^issue reopen" "$GH_LOG"
    grep -q "^issue comment 602 --repo test-owner/test-repo" "$GH_LOG"
}

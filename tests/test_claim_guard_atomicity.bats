#!/usr/bin/env bats
# tests/test_claim_guard_atomicity.bats — concurrency, overlap, stale reclaim,
# session identity, and degrade safety for claim-guard.sh.
#
# These are the counter-team (concurrency + portability) assertions:
#   - two concurrent acquires of ONE key grant exactly one owner (no TOCTOU
#     double-grant); the loser exits 6.
#   - an overlapping skill golden path conflicts with a held skill:<name>;
#     a disjoint path does not.
#   - a claim past TTL is reclaimable; one within TTL is never stolen.
#   - a different session is blocked; the same session is not.
#
# Real filesystem store, no mocks. Two "sessions" are simulated by toggling
# CLAUDE_CODE_SESSION_ID between subprocess invocations — that is exactly the
# stable per-session token claim-guard keys on.

bats_require_minimum_version 1.5.0

GUARD="${BATS_TEST_DIRNAME}/../scripts/claim-guard.sh"

setup() {
    STATE_DIR="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$STATE_DIR"
    export AUTOSPEC_REPO="berlinguyinca/autospec"
    export AUTOSPEC_CLAIM_GUARD="strict"
    export AUTOSPEC_HOST="testhost"
    CLAIM_ROOT="${STATE_DIR}/edit-claims/berlinguyinca__autospec"
}

teardown() {
    rm -rf "$STATE_DIR"
}

# ---------------------------------------------------------------------------
@test "code_health:claim_conflict identifier is present in the script" {
    grep -q 'code_health:claim_conflict' "$GUARD"
}

# ---------------------------------------------------------------------------
@test "a second session cannot acquire a key already held within TTL (exit 6)" {
    CLAUDE_CODE_SESSION_ID="sess-owner" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]

    CLAUDE_CODE_SESSION_ID="sess-intruder" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 6 ]
    printf '%s\n' "$output" | grep -q 'code_health:claim_conflict'
}

# ---------------------------------------------------------------------------
@test "assert by another session on a held key exits 6; the owner exits 0" {
    CLAUDE_CODE_SESSION_ID="sess-owner" bash "$GUARD" acquire skills/autospec-run

    CLAUDE_CODE_SESSION_ID="sess-intruder" run bash "$GUARD" assert skills/autospec-run
    [ "$status" -eq 6 ]

    CLAUDE_CODE_SESSION_ID="sess-owner" run bash "$GUARD" assert skills/autospec-run
    [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------------------
@test "two concurrent acquires of one key grant exactly one owner; the loser exits 6" {
    race_dir="$(mktemp -d)"
    out_a="${race_dir}/a.rc"
    out_b="${race_dir}/b.rc"

    # Launch two acquires racing for the same key from two distinct sessions.
    # Capture each PID and wait on it explicitly. NOTE: bats runs tests under
    # `set -e`, which the backgrounded subshells inherit — so the failing (exit
    # 6) acquire would abort the subshell before it could record its rc. Guard
    # the acquire with `|| rc=$?` so the rc is always written to the file.
    ( CLAUDE_CODE_SESSION_ID="sess-A" rc=0
      bash "$GUARD" acquire skills/autospec-run >/dev/null 2>&1 || rc=$?
      printf '%s' "$rc" > "$out_a" ) &
    pid_a=$!
    ( CLAUDE_CODE_SESSION_ID="sess-B" rc=0
      bash "$GUARD" acquire skills/autospec-run >/dev/null 2>&1 || rc=$?
      printf '%s' "$rc" > "$out_b" ) &
    pid_b=$!
    # `wait <pid>` returns the job's exit status; the loser exits 6, which would
    # abort this test under bats' set -e. Swallow it — we read rc from the files.
    wait "$pid_a" || true
    wait "$pid_b" || true

    rc_a="$(cat "$out_a")"
    rc_b="$(cat "$out_b")"
    rm -rf "$race_dir"

    # Exactly one winner (rc 0) and one loser (rc 6).
    winners=0
    [ "$rc_a" = "0" ] && winners=$((winners + 1)) || true
    [ "$rc_b" = "0" ] && winners=$((winners + 1)) || true
    [ "$winners" -eq 1 ]

    losers=0
    [ "$rc_a" = "6" ] && losers=$((losers + 1)) || true
    [ "$rc_b" = "6" ] && losers=$((losers + 1)) || true
    [ "$losers" -eq 1 ]

    # The persisted claim is owned by exactly one of the two sessions.
    claim_file="${CLAIM_ROOT}/skill-autospec-run.json"
    [ -f "$claim_file" ]
    owner="$(jq -r '.owner_session' "$claim_file")"
    case "$owner" in sess-A|sess-B) : ;; *) return 1 ;; esac
}

# ---------------------------------------------------------------------------
@test "multi-path acquire is all-or-nothing: a partial conflict releases the taken key" {
    # sess-owner already holds skill:autospec-explore.
    CLAUDE_CODE_SESSION_ID="sess-owner" bash "$GUARD" acquire skills/autospec-explore

    # sess-new tries to grab autospec-run AND autospec-explore atomically; the
    # explore key conflicts, so the whole acquire fails AND autospec-run must NOT
    # be left claimed by sess-new.
    CLAUDE_CODE_SESSION_ID="sess-new" run bash "$GUARD" \
        acquire skills/autospec-run skills/autospec-explore
    [ "$status" -eq 6 ]

    [ ! -f "${CLAIM_ROOT}/skill-autospec-run.json" ]
    # explore is still owned by the original owner.
    owner="$(jq -r '.owner_session' "${CLAIM_ROOT}/skill-autospec-explore.json")"
    [ "$owner" = "sess-owner" ]
}

# ---------------------------------------------------------------------------
@test "a partial-conflict rollback keeps a claim the SAME session already held" {
    # I already hold skill:autospec-run from a previous acquire.
    CLAUDE_CODE_SESSION_ID="sess-me" bash "$GUARD" acquire skills/autospec-run
    [ -f "${CLAIM_ROOT}/skill-autospec-run.json" ]

    # Another session holds skill:autospec-explore.
    CLAUDE_CODE_SESSION_ID="sess-other" bash "$GUARD" acquire skills/autospec-explore

    # I now try to acquire BOTH run (already mine) and explore (conflict). The
    # all-or-nothing rollback must NOT drop my pre-existing run claim.
    CLAUDE_CODE_SESSION_ID="sess-me" run bash "$GUARD" \
        acquire skills/autospec-run skills/autospec-explore
    [ "$status" -eq 6 ]

    [ -f "${CLAIM_ROOT}/skill-autospec-run.json" ]
    owner="$(jq -r '.owner_session' "${CLAIM_ROOT}/skill-autospec-run.json")"
    [ "$owner" = "sess-me" ]
}

# ---------------------------------------------------------------------------
@test "a crash-orphaned lock dir (no claim file) is reclaimable past TTL" {
    export AUTOSPEC_CLAIM_TTL_SECONDS=1
    mkdir -p "$CLAIM_ROOT"
    # Simulate a kill -9 between mkdir <lock> and write_claim: a .lock dir with
    # no backing JSON. Age it past TTL.
    orphan="${CLAIM_ROOT}/skill-autospec-run.lock"
    mkdir -p "$orphan"
    # Backdate the lock dir mtime well past the 1s TTL (portable touch -t).
    touch -t 200001010000 "$orphan" 2>/dev/null || true

    CLAUDE_CODE_SESSION_ID="sess-fresh" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
    [ -f "${CLAIM_ROOT}/skill-autospec-run.json" ]
    owner="$(jq -r '.owner_session' "${CLAIM_ROOT}/skill-autospec-run.json")"
    [ "$owner" = "sess-fresh" ]
}

# ---------------------------------------------------------------------------
@test "an overlapping skill golden path conflicts with a held skill:<name> claim" {
    CLAUDE_CODE_SESSION_ID="sess-owner" bash "$GUARD" acquire skills/autospec-run

    # The golden of the same skill resolves to the same lock key -> conflict.
    CLAUDE_CODE_SESSION_ID="sess-other" run bash "$GUARD" \
        assert "tests/fixtures/skill-goldens/autospec-run.sha256"
    [ "$status" -eq 6 ]

    # A disjoint skill does NOT conflict.
    CLAUDE_CODE_SESSION_ID="sess-other" run bash "$GUARD" assert skills/autospec-explore
    [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------------------
@test "stale reclaim: a claim past TTL is reclaimable by another session" {
    export AUTOSPEC_CLAIM_TTL_SECONDS=1
    CLAUDE_CODE_SESSION_ID="sess-old" bash "$GUARD" acquire skills/autospec-run
    claim_file="${CLAIM_ROOT}/skill-autospec-run.json"

    # Age the claim past TTL by rewriting updated_at far into the past.
    tmp="$(mktemp)"
    jq '.updated_at = "2000-01-01T00:00:00Z"' "$claim_file" > "$tmp" && mv "$tmp" "$claim_file"

    CLAUDE_CODE_SESSION_ID="sess-fresh" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
    owner="$(jq -r '.owner_session' "$claim_file")"
    [ "$owner" = "sess-fresh" ]
}

# ---------------------------------------------------------------------------
@test "stale reclaim never steals a live claim within TTL" {
    export AUTOSPEC_CLAIM_TTL_SECONDS=1800
    CLAUDE_CODE_SESSION_ID="sess-live" bash "$GUARD" acquire skills/autospec-run

    # Claim is fresh (updated_at = now), well within TTL -> intruder is blocked.
    CLAUDE_CODE_SESSION_ID="sess-thief" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 6 ]
    owner="$(jq -r '.owner_session' "${CLAIM_ROOT}/skill-autospec-run.json")"
    [ "$owner" = "sess-live" ]
}

# ---------------------------------------------------------------------------
@test "status flags a stale claim" {
    export AUTOSPEC_CLAIM_TTL_SECONDS=1
    CLAUDE_CODE_SESSION_ID="sess-old" bash "$GUARD" acquire skills/autospec-run
    claim_file="${CLAIM_ROOT}/skill-autospec-run.json"
    tmp="$(mktemp)"
    jq '.updated_at = "2000-01-01T00:00:00Z"' "$claim_file" > "$tmp" && mv "$tmp" "$claim_file"

    run bash "$GUARD" status
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -qi 'stale'
}

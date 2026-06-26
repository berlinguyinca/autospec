#!/usr/bin/env bats
# tests/test_claim_guard_session.bats — session-identity / nested-claim safety.
#
# This is the autospec-run Phase 4 integration (#1067) contract test: the
# implementer `acquire`s the edit surface AFTER it already holds the issue-level
# lease and INSIDE its own worktree, then the same session may legitimately
# touch that surface repeatedly (re-acquire / refresh) across the edit+test+PR
# step. So:
#   - the SAME session re-acquiring its OWN live claim is an idempotent no-op
#     success (the nested claim never deadlocks against itself);
#   - a DIFFERENT session is blocked (exit 6) while the first holds the lease,
#     which is exactly what prevents two worktrees stomping one trio+golden set.
#
# Real filesystem store, no mocks (AGENTS.md). The two sessions are simulated by
# toggling CLAUDE_CODE_SESSION_ID between subprocess invocations — the stable
# per-session token claim-guard keys on. The store root is redirected via
# AUTOSPEC_STATE_DIR so we never touch ~/.autospec.

bats_require_minimum_version 1.5.0

GUARD="${BATS_TEST_DIRNAME}/../scripts/claim-guard.sh"

setup() {
    STATE_DIR="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$STATE_DIR"
    # Deterministic repo identity so the slug is stable and assertable without
    # re-deriving it from the SUT (self-consistent-fixture trap).
    export AUTOSPEC_REPO="berlinguyinca/autospec"
    export AUTOSPEC_CLAIM_GUARD="strict"
    export AUTOSPEC_HOST="testhost"
    # Hold the claim comfortably within TTL for the whole test so no stale
    # reclaim can fire and mask a session-identity decision.
    export AUTOSPEC_CLAIM_TTL_SECONDS=1800
    CLAIM_ROOT="${STATE_DIR}/edit-claims/berlinguyinca__autospec"
}

teardown() {
    rm -rf "$STATE_DIR"
}

# ---------------------------------------------------------------------------
@test "same session re-acquiring its own live claim is an idempotent no-op success" {
    CLAUDE_CODE_SESSION_ID="sess-self" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]

    # Re-acquire the very same key from the very same session: must succeed.
    CLAUDE_CODE_SESSION_ID="sess-self" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]

    # And still owned by that one session (no duplicate / no orphan).
    claim_file="${CLAIM_ROOT}/skill-autospec-run.json"
    [ -f "$claim_file" ]
    owner="$(jq -r '.owner_session' "$claim_file")"
    [ "$owner" = "sess-self" ]
}

# ---------------------------------------------------------------------------
@test "same session re-acquiring across the whole trio+golden surface is a no-op" {
    # The autospec-run integration acquires the skill surface, then the same
    # session edits SKILL.md, codex/prompt.md, opencode/agent.md AND the goldens
    # — all of which resolve to skill:autospec-run. Re-acquiring any of them must
    # be safe for the holding session (nested-claim safety, not a self-deadlock).
    CLAUDE_CODE_SESSION_ID="sess-self" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]

    CLAUDE_CODE_SESSION_ID="sess-self" run bash "$GUARD" \
        acquire "skills/autospec-run/codex/prompt.md"
    [ "$status" -eq 0 ]

    CLAUDE_CODE_SESSION_ID="sess-self" run bash "$GUARD" \
        acquire "tests/fixtures/skill-goldens/autospec-run.SKILL.md.sha256"
    [ "$status" -eq 0 ]

    # Still exactly one claim, owned by the one session.
    claim_file="${CLAIM_ROOT}/skill-autospec-run.json"
    [ -f "$claim_file" ]
    owner="$(jq -r '.owner_session' "$claim_file")"
    [ "$owner" = "sess-self" ]
}

# ---------------------------------------------------------------------------
@test "a different session is blocked from acquiring a held claim (exit 6)" {
    CLAUDE_CODE_SESSION_ID="sess-owner" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]

    CLAUDE_CODE_SESSION_ID="sess-other" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 6 ]
    printf '%s\n' "$output" | grep -q 'code_health:claim_conflict'

    # The original owner still holds it; the intruder did not overwrite it.
    owner="$(jq -r '.owner_session' "${CLAIM_ROOT}/skill-autospec-run.json")"
    [ "$owner" = "sess-owner" ]
}

# ---------------------------------------------------------------------------
@test "after the owner releases, a different session can acquire (lease handoff)" {
    CLAUDE_CODE_SESSION_ID="sess-owner" bash "$GUARD" acquire skills/autospec-run
    CLAUDE_CODE_SESSION_ID="sess-owner" run bash "$GUARD" release skills/autospec-run
    [ "$status" -eq 0 ]
    [ ! -f "${CLAIM_ROOT}/skill-autospec-run.json" ]

    CLAUDE_CODE_SESSION_ID="sess-next" run bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
    owner="$(jq -r '.owner_session' "${CLAIM_ROOT}/skill-autospec-run.json")"
    [ "$owner" = "sess-next" ]
}

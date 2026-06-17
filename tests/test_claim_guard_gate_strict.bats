#!/usr/bin/env bats
# tests/test_claim_guard_gate_strict.bats — issue #1073.
#
# The autospec-run Phase 4 gate wires `acquire $TARGETS` as a BLOCKING `if !`
# gate (exit 6 -> restore auto-implement + stop). But `acquire` only exits 6
# under AUTOSPEC_CLAIM_GUARD=strict; the shipped default is `warn` (exit 0 on
# conflict). The fix scopes the gate's acquire call to strict so the blocking
# branch actually fires, while leaving the GLOBAL default `warn` untouched for
# interactive/manual use.
#
# These tests pin BOTH halves of that contract:
#   1. Under the bare default (no AUTOSPEC_CLAIM_GUARD), a contested acquire
#      still exits 0 — so the gate would be dead code if it relied on the
#      default. (Regression guard: the global default must stay warn.)
#   2. With AUTOSPEC_CLAIM_GUARD=strict scoped to the call, a contested acquire
#      exits 6 — so the gate's `if !` blocking branch fires.
#   3. The SKILL.md gate text actually scopes the acquire call to strict (the
#      wiring fix), so the live skill prose matches the blocking semantics.
#
# Real filesystem store; AUTOSPEC_STATE_DIR redirected to a per-test tmp dir.
# No mocks (AGENTS.md).

bats_require_minimum_version 1.5.0

GUARD="${BATS_TEST_DIRNAME}/../scripts/claim-guard.sh"
REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
    STATE_DIR="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$STATE_DIR"
    export AUTOSPEC_REPO="berlinguyinca/autospec"
    export CLAUDE_CODE_SESSION_ID="sess-gate-strict-aaaa"
    export AUTOSPEC_HOST="testhost"
    CLAIM_ROOT="${STATE_DIR}/edit-claims/berlinguyinca_autospec"
}

teardown() {
    rm -rf "$STATE_DIR"
}

# Plant a live claim for skill:autospec-run owned by a DIFFERENT session.
plant_other_session_claim() {
    mkdir -p "$CLAIM_ROOT"
    local now
    now="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
    cat > "${CLAIM_ROOT}/skill-autospec-run.json" <<EOF
{
  "lock_key": "skill:autospec-run",
  "paths": ["skills/autospec-run/"],
  "owner_session": "sess-other-zzzz",
  "host": "otherhost",
  "pid": 9999,
  "branch": "feat/other-branch",
  "worktree": "/tmp/other-wt",
  "acquired_at": "${now}",
  "updated_at": "${now}",
  "ttl_seconds": 1800
}
EOF
    mkdir -p "${CLAIM_ROOT}/skill-autospec-run.lock"
}

# ---------------------------------------------------------------------------
@test "contested acquire under the bare default (warn) exits 0 — gate must not rely on default" {
    plant_other_session_claim
    # No AUTOSPEC_CLAIM_GUARD set: the shipped global default is warn.
    run env -u AUTOSPEC_CLAIM_GUARD bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------------------
@test "contested acquire scoped to strict exits 6 — the blocking gate fires" {
    plant_other_session_claim
    run env AUTOSPEC_CLAIM_GUARD=strict bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 6 ]
}

# ---------------------------------------------------------------------------
@test "uncontested acquire scoped to strict still exits 0 (no false block)" {
    run env AUTOSPEC_CLAIM_GUARD=strict bash "$GUARD" acquire skills/autospec-run
    [ "$status" -eq 0 ]
    [ -f "${CLAIM_ROOT}/skill-autospec-run.json" ]
}

# ---------------------------------------------------------------------------
@test "SKILL.md gate scopes the acquire call to AUTOSPEC_CLAIM_GUARD=strict" {
    # The blocking `if ! ... acquire $TARGETS` line must scope strict to the
    # call so the gate fires under the global warn default. Pin all three trio
    # members + the phase4-implementer prompt.
    for f in skills/autospec-run/SKILL.md \
             skills/autospec-run/codex/prompt.md \
             skills/autospec-run/opencode/agent.md; do
        run grep -qE 'if ! +AUTOSPEC_CLAIM_GUARD=strict +bash .*claim-guard\.sh acquire \$TARGETS' \
            "${REPO_ROOT}/${f}"
        [ "$status" -eq 0 ] || { echo "missing strict-scoped gate in ${f}" >&2; return 1; }
    done
    run grep -qE 'AUTOSPEC_CLAIM_GUARD=strict +bash .*claim-guard\.sh acquire \$TARGETS' \
        "${REPO_ROOT}/skills/autospec-run/prompts/phase4-implementer.md"
    [ "$status" -eq 0 ]
}

#!/usr/bin/env bats
# tests/unit/test_validate_derive_trio.bats — exercise the
# check_derive_trio_consistency() gate added to scripts/validate.sh
# (trio-derivation Phase 2, decomposition children C + E).
#
# Contract:
#   * The function exists and is wired into main()'s global check list.
#   * It passes against the real repo as-is (every trio's --check is green —
#     this is the "migration/adoption" assertion: derive-trio is now canonical).
#   * When a trio mirror (codex/prompt.md or opencode/agent.md) is hand-edited
#     out of sync, validate.sh fails AND the diagnostic names the skill and the
#     exact fix command:
#       run: scripts/derive-trio.sh --in-place skills/<name> && \
#            scripts/gen-skill-goldens.sh <name>

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    VALIDATE="$REPO_ROOT/scripts/validate.sh"
    SCRATCH="$(mktemp -d)"
    export SCRATCH
}

teardown() {
    if [ -n "${SCRATCH:-}" ] && [ -d "$SCRATCH" ]; then
        rm -rf "$SCRATCH"
    fi
}

# ---- presence / wiring ---------------------------------------------------

@test "validate.sh: defines check_derive_trio_consistency" {
    run grep -q '^check_derive_trio_consistency()' "$VALIDATE"
    [ "$status" -eq 0 ]
}

@test "validate.sh: wires check_derive_trio_consistency into main()" {
    run grep -Eq '^[[:space:]]+check_derive_trio_consistency$' "$VALIDATE"
    [ "$status" -eq 0 ]
}

# ---- clean tree passes ---------------------------------------------------

@test "validate.sh: passes against the real repo as-is (derive-trio --check green for all trios)" {
    run bash "$VALIDATE"
    [ "$status" -eq 0 ]
}

# ---- drift in a trio mirror fails with the friendly fix command ----------
#
# check_derive_trio_consistency is exercised IN ISOLATION here. In the full
# main() run order the per-skill check_lockstep byte-diff fires first and exits
# on drift, so it would shadow this check's diagnostic; that shadowing is fine
# (both enforce the same invariant) — this gate's job is to emit the actionable
# single-command fix, which we assert by driving the function directly against a
# drifted scratch skills/ tree. We source the function out of validate.sh so the
# test tracks the real implementation (no copy-paste) and provide the fail/info
# helpers it depends on, with `fail` writing the message to a real temp file
# (bash-3.2 safe; no process-substitution into [ -f ]).

# Build a hermetic skills/ tree containing one real trio under $SCRATCH and run
# check_derive_trio_consistency from there. Captures combined output; returns the
# function's exit status. The fail message lands in $SCRATCH/fail.out.
_run_derive_check_in_scratch() {
    mkdir -p "$SCRATCH/skills" "$SCRATCH/scripts"
    cp -R "$REPO_ROOT/skills/autospec-explore" "$SCRATCH/skills/autospec-explore"
    cp "$REPO_ROOT/scripts/derive-trio.sh" "$SCRATCH/scripts/derive-trio.sh"
    # Extract the function body verbatim from validate.sh (def line to its closing
    # brace at column 0) so the test asserts the SHIPPED implementation.
    awk '/^check_derive_trio_consistency\(\) \{/{p=1} p{print} p&&/^\}/{exit}' \
        "$VALIDATE" > "$SCRATCH/fn.sh"
    (
        cd "$SCRATCH"
        # shellcheck disable=SC1091
        set +e
        fail() { printf 'validate: FAIL — %s\n' "$*" > "$SCRATCH/fail.out"; cat "$SCRATCH/fail.out" >&2; exit 1; }
        info() { printf 'validate: %s\n' "$*"; }
        discover_skills() { for d in skills/*/; do
            [ -d "$d" ] || continue
            if [ -f "$d/SKILL.md" ] && [ -f "$d/opencode/agent.md" ] && [ -f "$d/codex/prompt.md" ]; then
                printf '%s\n' "${d%/}"
            fi
        done; }
        # shellcheck source=/dev/null
        . "$SCRATCH/fn.sh"
        check_derive_trio_consistency
    )
}

@test "check_derive_trio_consistency: passes on a clean scratch trio" {
    run _run_derive_check_in_scratch
    [ "$status" -eq 0 ]
}

# ---- valid cross-dir: resolves trio paths via its own discovery -----------
#
# check_derive_trio_consistency() must not assume validate.sh was invoked from
# the repo root: it resolves the trio via discover_skills globbing skills/*/
# relative to its own CWD plus a relative scripts/derive-trio.sh, never an
# absolute repo path. _run_derive_check_in_scratch cd's into $SCRATCH (created
# by mktemp -d, distinct from $REPO_ROOT) and runs the check there against a
# clean hermetic trio, so a green exit proves the cross-dir resolution. We also
# assert no fail.out was written — the absence of the fail diagnostic confirms
# the check found and passed the scratch trio rather than erroring on a missing
# absolute path.
@test "check_derive_trio_consistency: passes when run from a working dir other than the repo root (valid cross-dir)" {
    # Guard: the scratch CWD the helper uses must differ from the repo root, so
    # a green result genuinely exercises the cross-dir path-resolution behavior.
    [ "$SCRATCH" != "$REPO_ROOT" ]
    run _run_derive_check_in_scratch
    [ "$status" -eq 0 ]
    [ ! -f "$SCRATCH/fail.out" ]
}

@test "check_derive_trio_consistency: fails with the fix command when a codex mirror drifts" {
    mkdir -p "$SCRATCH/skills" "$SCRATCH/scripts"
    cp -R "$REPO_ROOT/skills/autospec-explore" "$SCRATCH/skills/autospec-explore"
    cp "$REPO_ROOT/scripts/derive-trio.sh" "$SCRATCH/scripts/derive-trio.sh"
    printf '\nHAND-EDITED DRIFT LINE\n' >> "$SCRATCH/skills/autospec-explore/codex/prompt.md"
    run _run_derive_check_in_scratch
    [ "$status" -ne 0 ]
    [ -f "$SCRATCH/fail.out" ]
    grep -F 'autospec-explore' "$SCRATCH/fail.out" >/dev/null
    grep -F 'scripts/derive-trio.sh --in-place skills/autospec-explore' "$SCRATCH/fail.out" >/dev/null
    grep -F 'scripts/gen-skill-goldens.sh autospec-explore' "$SCRATCH/fail.out" >/dev/null
}

@test "check_derive_trio_consistency: fails with the fix command when an opencode mirror drifts" {
    mkdir -p "$SCRATCH/skills" "$SCRATCH/scripts"
    cp -R "$REPO_ROOT/skills/autospec-explore" "$SCRATCH/skills/autospec-explore"
    cp "$REPO_ROOT/scripts/derive-trio.sh" "$SCRATCH/scripts/derive-trio.sh"
    printf '\nHAND-EDITED OPENCODE DRIFT\n' >> "$SCRATCH/skills/autospec-explore/opencode/agent.md"
    run _run_derive_check_in_scratch
    [ "$status" -ne 0 ]
    [ -f "$SCRATCH/fail.out" ]
    grep -F 'autospec-explore' "$SCRATCH/fail.out" >/dev/null
    grep -F 'scripts/derive-trio.sh --in-place skills/autospec-explore' "$SCRATCH/fail.out" >/dev/null
}

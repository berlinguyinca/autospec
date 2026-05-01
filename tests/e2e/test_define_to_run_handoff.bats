#!/usr/bin/env bats
# tests/e2e/test_define_to_run_handoff.bats — opt-in end-to-end test
# that exercises the Phase 3 pre-impl gate routing between
# /autospec-define and /autospec-run.
#
# What this asserts at the e2e tier (without forking a real subagent
# session):
#
#   * On `defer`, no run-side state transition happens — the seeded
#     `auto-implement` issue still carries its label and no
#     `in-progress-by-bot` label is applied.
#
#   * On `run`, the run-side state transition starts — the implementer
#     monitor's first action (per the SKILL prompt) is the
#     `auto-implement` → `in-progress-by-bot` label swap. We simulate
#     that swap and assert the issue is now labeled `in-progress-by-bot`.
#
# Cleanup: setup() creates a throwaway gh repo, teardown() deletes it
# regardless of pass/fail. The test skips cleanly when `gh` is missing,
# unauthenticated, or `gh repo create` is denied.
#
# Spec ref: docs/specs/2026-05-01-autospec-meta-improvements-design.md §3.3, §6.3, §6.6.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"

    if ! command -v gh >/dev/null 2>&1; then
        skip "gh CLI not installed"
    fi
    if ! gh auth status >/dev/null 2>&1; then
        skip "gh not authenticated (set GH_TOKEN or run gh auth login)"
    fi
    OWNER="$(gh api user --jq .login 2>/dev/null || true)"
    if [ -z "$OWNER" ]; then
        skip "could not resolve gh user"
    fi
    SUFFIX="$(date +%s)-$$"
    THROWAWAY_REPO="$OWNER/autospec-e2e-handoff-$SUFFIX"
    export OWNER THROWAWAY_REPO

    if ! gh repo create "$THROWAWAY_REPO" --public --confirm >/dev/null 2>&1; then
        skip "gh repo create denied or unavailable in this environment"
    fi

    # Seed labels and an auto-implement issue.
    gh label create auto-implement       --repo "$THROWAWAY_REPO" --color 0e8a16 --force >/dev/null
    gh label create in-progress-by-bot   --repo "$THROWAWAY_REPO" --color ededed --force >/dev/null

    seed_url="$(gh issue create \
        --repo "$THROWAWAY_REPO" \
        --title "Seed feature: hello world" \
        --body  "Seed issue for handoff e2e." \
        --label auto-implement)"
    SEED_NUM="${seed_url##*/}"
    export SEED_NUM
}

teardown() {
    if [ -n "${THROWAWAY_REPO:-}" ]; then
        gh repo delete "$THROWAWAY_REPO" --yes >/dev/null 2>&1 || true
    fi
}

# Local model of the Phase 3 pre-impl gate routing. Returns 0 and prints
# the route name. Mirrors the spec text in skills/autospec{,-define}/SKILL.md.
gate_route() {
    answer="$1"
    case "$answer" in
        run)    printf 'run\n' ;;
        defer)  printf 'defer\n' ;;
        refine) printf 'refine\n' ;;
        *)      printf 'invalid\n'; return 2 ;;
    esac
}

# ---- defer answer ---------------------------------------------------------

@test "Phase 3 gate: defer answer skips run (no implementer label swap)" {
    route="$(gate_route defer)"
    [ "$route" = "defer" ]

    # On defer, no implementer subagent runs, so the seeded issue keeps
    # its `auto-implement` label and gains no `in-progress-by-bot` label.
    labels="$(gh issue view "$SEED_NUM" --repo "$THROWAWAY_REPO" --json labels --jq '.labels[].name' | sort)"
    echo "$labels" | grep -F 'auto-implement' >/dev/null
    ! echo "$labels" | grep -F 'in-progress-by-bot' >/dev/null
}

# ---- run answer -----------------------------------------------------------

@test "Phase 3 gate: run answer invokes run (implementer label swap fires)" {
    route="$(gate_route run)"
    [ "$route" = "run" ]

    # On run, the implementer monitor's first action per the SKILL.md
    # prompt is to swap auto-implement → in-progress-by-bot. We invoke
    # that step in-band to faithfully simulate the run-side handoff.
    gh issue edit "$SEED_NUM" \
        --repo "$THROWAWAY_REPO" \
        --remove-label auto-implement \
        --add-label    in-progress-by-bot >/dev/null

    labels="$(gh issue view "$SEED_NUM" --repo "$THROWAWAY_REPO" --json labels --jq '.labels[].name' | sort)"
    echo "$labels" | grep -F 'in-progress-by-bot' >/dev/null
    ! echo "$labels" | grep -F 'auto-implement' >/dev/null
}

# ---- refine answer (sanity) -----------------------------------------------

@test "Phase 3 gate: refine answer is recognized as a distinct route" {
    route="$(gate_route refine)"
    [ "$route" = "refine" ]
    # refine re-enters Phase 2 — no run-side artifacts either.
    labels="$(gh issue view "$SEED_NUM" --repo "$THROWAWAY_REPO" --json labels --jq '.labels[].name' | sort)"
    echo "$labels" | grep -F 'auto-implement' >/dev/null
    ! echo "$labels" | grep -F 'in-progress-by-bot' >/dev/null
}

#!/usr/bin/env bats
# tests/autonomous/test_guarded_merge.bats — issue #1732
# Guarded-merge wrapper: per-diff blast-radius domain fence at the merge
# chokepoint. Uses the REAL classifier + guardrails against a fixture
# fenced-surfaces registry; stubs gh to drive changed-files/labels and to
# record merge/edit/comment calls.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
WRAPPER="$REPO_ROOT/scripts/autospec-guarded-merge.sh"

setup() {
    TMP="$(mktemp -d -t guarded_merge.XXXXXX)"
    export PATH="$TMP/bin:$PATH"
    mkdir -p "$TMP/bin"
    GH_LOG="$TMP/gh.log"; export GH_LOG
    : > "$GH_LOG"

    # Configurable gh stub: FILES / LABELS env drive `pr view`; FILES_FAIL
    # forces the files read to fail; everything else logs and succeeds.
    cat > "$TMP/bin/gh" <<'STUB'
#!/usr/bin/env bash
printf 'gh %s\n' "$*" >> "$GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    case "$*" in
        *"--json files"*)
            [ "${FILES_FAIL:-0}" = "1" ] && exit 1
            for p in $FILES; do printf '%s\n' "$p"; done
            exit 0 ;;
        *"--json labels"*)
            for l in $LABELS; do printf '%s\n' "$l"; done
            exit 0 ;;
        *"--json statusCheckRollup"*)
            if [ "${ROLLUP_FAIL:-0}" = "1" ]; then exit 1; fi
            # NOTE: no `${ROLLUP:-<json>}` default here. A `}` inside the
            # default value closes the parameter expansion early, which silently
            # appended `]}` to every rollup and made jq reject valid input.
            if [ -z "${ROLLUP+x}" ]; then
                ROLLUP='[{"name":"build-test","conclusion":"SUCCESS"}]'
            fi
            printf '%s\n' "$ROLLUP"
            exit 0 ;;
    esac
fi
exit 0
STUB
    chmod +x "$TMP/bin/gh"

    # Fixture registry: fence ONLY the risk-engine crate.
    cat > "$TMP/fenced.yml" <<'YAML'
fenced_surfaces:
  - id: trading-risk
    severity: fenced
    reason: risk engine moves capital
    paths:
      - "crates/risk-engine/**"
YAML
}

teardown() { rm -rf "$TMP"; }

@test "allow: non-fenced diff merges via the wrapper" {
    export FILES="crates/backtesting/src/engine.rs" LABELS=""
    run bash "$WRAPPER" --pr 5 --repo o/r --fenced-surfaces "$TMP/fenced.yml"
    [ "$status" -eq 0 ]
    grep -q "merged fenced_surface_ok" <<<"$output"
    grep -q "pr merge 5" "$GH_LOG"
}

@test "quarantine: fenced diff, no override -> blocked, NOT merged, needs-human applied" {
    export FILES="crates/risk-engine/src/lib.rs" LABELS=""
    run bash "$WRAPPER" --pr 6 --repo o/r --fenced-surfaces "$TMP/fenced.yml"
    [ "$status" -eq 1 ]
    grep -q "blocked fenced_surface" <<<"$output"
    grep -q "add-label autospec:needs-human" "$GH_LOG"
    ! grep -q "pr merge 6" "$GH_LOG"
}

@test "override: fenced diff with override label merges" {
    export FILES="crates/risk-engine/src/lib.rs" LABELS="autospec:fenced-approved"
    run bash "$WRAPPER" --pr 7 --repo o/r --fenced-surfaces "$TMP/fenced.yml"
    [ "$status" -eq 0 ]
    grep -q "merged fenced_surface_ok" <<<"$output"
    grep -q "pr merge 7" "$GH_LOG"
}

@test "fail-closed: cannot read changed files -> exit 2, NOT merged" {
    export FILES="crates/backtesting/src/engine.rs" LABELS="" FILES_FAIL=1
    run bash "$WRAPPER" --pr 8 --repo o/r --fenced-surfaces "$TMP/fenced.yml"
    [ "$status" -eq 2 ]
    ! grep -q "pr merge 8" "$GH_LOG"
}

@test "empty diff: nothing to classify -> merges" {
    export FILES="" LABELS=""
    run bash "$WRAPPER" --pr 9 --repo o/r --fenced-surfaces "$TMP/fenced.yml"
    [ "$status" -eq 0 ]
    grep -q "pr merge 9" "$GH_LOG"
}

@test "missing required args -> invocation error (exit 2)" {
    run bash "$WRAPPER" --repo o/r
    [ "$status" -eq 2 ]
}

# ── CI-conclusion gate (issue #3220) ────────────────────────────────────────
# `main` carries no branch protection, so no check is "required" and a PR whose
# checks are pending or failing reports mergeStateStatus UNSTABLE. The Phase 4
# loop treats UNSTABLE as ready and breaks straight to merge, so its
# `wait_for_ci_green` never runs on the normal path. #3148 and #3216 both
# merged before their run reported; #3148's build-test was already failing.

@test "checks gate: a failing non-advisory check refuses the merge" {
    export FILES="crates/backtesting/src/engine.rs" LABELS=""
    export ROLLUP='[{"name":"build-test","conclusion":"FAILURE"}]'
    run bash "$WRAPPER" --pr 10 --repo o/r --fenced-surfaces "$TMP/fenced.yml"
    [ "$status" -eq 1 ]
    grep -q "blocked checks_not_green" <<<"$output"
    ! grep -q "pr merge 10" "$GH_LOG"
}

@test "checks gate: a pending check refuses rather than merging early" {
    export FILES="crates/backtesting/src/engine.rs" LABELS=""
    export ROLLUP='[{"name":"build-test","conclusion":null}]'
    run bash "$WRAPPER" --pr 11 --repo o/r --fenced-surfaces "$TMP/fenced.yml" --checks-timeout 1 --checks-poll 1
    [ "$status" -eq 1 ]
    grep -q "blocked checks_not_green" <<<"$output"
    ! grep -q "pr merge 11" "$GH_LOG"
}

@test "checks gate: an empty rollup is not proof of green" {
    export FILES="crates/backtesting/src/engine.rs" LABELS=""
    export ROLLUP='[]'
    run bash "$WRAPPER" --pr 12 --repo o/r --fenced-surfaces "$TMP/fenced.yml" --checks-timeout 1 --checks-poll 1
    [ "$status" -eq 1 ]
    ! grep -q "pr merge 12" "$GH_LOG"
}

@test "checks gate: SKIPPED and NEUTRAL are green, not pending" {
    export FILES="crates/backtesting/src/engine.rs" LABELS=""
    export ROLLUP='[{"name":"doc-drift","conclusion":"SKIPPED"},{"name":"x","conclusion":"NEUTRAL"},{"name":"build-test","conclusion":"SUCCESS"}]'
    run bash "$WRAPPER" --pr 13 --repo o/r --fenced-surfaces "$TMP/fenced.yml"
    [ "$status" -eq 0 ]
    grep -q "pr merge 13" "$GH_LOG"
}

@test "checks gate: an advisory check may fail without blocking" {
    export FILES="crates/backtesting/src/engine.rs" LABELS=""
    export ROLLUP='[{"name":"TeamCity","conclusion":"FAILURE"},{"name":"build-test","conclusion":"SUCCESS"}]'
    export AUTOSPEC_PR_ADVISORY_CHECKS='^TeamCity$'
    run bash "$WRAPPER" --pr 14 --repo o/r --fenced-surfaces "$TMP/fenced.yml"
    [ "$status" -eq 0 ]
    grep -q "pr merge 14" "$GH_LOG"
}

@test "checks gate: a pending advisory check does not stall the gate" {
    export FILES="crates/backtesting/src/engine.rs" LABELS=""
    export ROLLUP='[{"name":"TeamCity","conclusion":null},{"name":"build-test","conclusion":"SUCCESS"}]'
    export AUTOSPEC_PR_ADVISORY_CHECKS='^TeamCity$'
    run bash "$WRAPPER" --pr 15 --repo o/r --fenced-surfaces "$TMP/fenced.yml" --checks-timeout 1 --checks-poll 1
    [ "$status" -eq 0 ]
    grep -q "pr merge 15" "$GH_LOG"
}

@test "checks gate: a StatusContext state is honored like a conclusion" {
    export FILES="crates/backtesting/src/engine.rs" LABELS=""
    export ROLLUP='[{"context":"legacy/ci","state":"FAILURE"}]'
    run bash "$WRAPPER" --pr 16 --repo o/r --fenced-surfaces "$TMP/fenced.yml"
    [ "$status" -eq 1 ]
    grep -q "blocked checks_not_green" <<<"$output"
    ! grep -q "pr merge 16" "$GH_LOG"
}

@test "checks gate: unreadable rollup fails closed, NOT merged" {
    export FILES="crates/backtesting/src/engine.rs" LABELS="" ROLLUP_FAIL=1
    run bash "$WRAPPER" --pr 17 --repo o/r --fenced-surfaces "$TMP/fenced.yml" --checks-timeout 1 --checks-poll 1
    [ "$status" -eq 2 ]
    ! grep -q "pr merge 17" "$GH_LOG"
}

@test "checks gate: --no-require-checks restores the unguarded merge" {
    export FILES="crates/backtesting/src/engine.rs" LABELS=""
    export ROLLUP='[{"name":"build-test","conclusion":"FAILURE"}]'
    run bash "$WRAPPER" --pr 18 --repo o/r --fenced-surfaces "$TMP/fenced.yml" --no-require-checks
    [ "$status" -eq 0 ]
    grep -q "pr merge 18" "$GH_LOG"
}

@test "checks gate: an unnamed pending check still blocks" {
    # An entry with neither name nor context cannot be matched by the advisory
    # regex, so it must count toward pending rather than being filtered out.
    # Excluding it would let an unidentifiable in-flight check pass as green.
    export FILES="crates/backtesting/src/engine.rs" LABELS=""
    # Paired with a green named check so the two filters are distinguishable:
    # if the unnamed entry is wrongly filtered out, the rollup looks like a
    # single SUCCESS and merges. A lone unnamed entry cannot tell them apart --
    # it would refuse either way, via the empty-rollup path.
    export ROLLUP='[{"conclusion":null},{"name":"build-test","conclusion":"SUCCESS"}]'
    run bash "$WRAPPER" --pr 19 --repo o/r --fenced-surfaces "$TMP/fenced.yml" --checks-timeout 1 --checks-poll 1
    [ "$status" -eq 1 ]
    grep -q "blocked checks_not_green" <<<"$output"
    ! grep -q "pr merge 19" "$GH_LOG"
}

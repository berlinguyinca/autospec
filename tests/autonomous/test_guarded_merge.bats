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

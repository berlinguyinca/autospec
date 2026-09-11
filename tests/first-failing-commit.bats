#!/usr/bin/env bats
# tests/first-failing-commit.bats — issue #4108 AC3: automated
# first-failing-commit report. Stubs `gh` and drives
# scripts/first-failing-commit.sh against canned `actions/runs` payloads.
# (Suite lives at the tests/ root, so no catalog registration is needed.)
#
# Two host-compatibility fixes (issue #4199 work):
#   - BATS_TEST_TMPDIR instead of mktemp+`trap ... EXIT`: an EXIT trap
#     installed in setup() overwrites bats' own EXIT trap, which is what
#     emits the `not ok` TAP line — a failing test then vanishes from the
#     output instead of being reported. This pattern silently masked test 1
#     on every host (the stub below also had a bad substitution there).
#   - The stub's sha extraction uses two steps: `${${x}%% *}` (a nested
#     expansion in the parameter slot) is a bad substitution in bash, so
#     LAST_GOOD_SUBJECT/FIRST_FAILING_SUBJECT could never be captured.

setup() {
    BIN="$BATS_TEST_TMPDIR/bin"
    mkdir -p "$BIN"
    GH_LOG="$BATS_TEST_TMPDIR/gh.log"
    : >"$GH_LOG"
    cat >"$BIN/gh" <<EOF
#!/usr/bin/env bash
printf 'gh %s\n' "\$*" >> "$GH_LOG"
case "\$*" in
  *"actions/runs?branch=main"*)
    if [ -n "\${RUNS_FILE:-}" ] && [ -f "\$RUNS_FILE" ]; then
      cat "\$RUNS_FILE"
      exit 0
    fi
    if [ -n "\${RUNS_FAIL:-}" ]; then
      exit 1
    fi
    printf '{}'
    ;;
  *"commits/"*)
    rest="\${*##*commits/}"
    sha="\${rest%% *}"
    printf '%s\n' "\${SUBJ:-subject of \$sha}"
    ;;
  *)
    printf '{}'
    ;;
esac
EOF
    chmod +x "$BIN/gh"
    PATH="$BIN:$PATH"
    export PATH
    SCRIPT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)/scripts/first-failing-commit.sh"
}

make_run() {
    # make_run <sha> <created_at> <conclusion>
    printf '{"head_sha":"%s","created_at":"%s","jobs":[{"name":"main-builds","conclusion":"%s","created_at":"%s"}]}' \
        "$1" "$2" "$3" "$2"
}

write_payload() {
    # write_payload <runs...> — runs are pre-built JSON objects, comma-joined
    printf '{"workflow_runs":[%s]}\n' "$1" >"$BATS_TEST_TMPDIR/runs.json"
    RUNS_FILE="$BATS_TEST_TMPDIR/runs.json"
    export RUNS_FILE
}

@test "broken: newest success and oldest failure after it are reported" {
    R1="$(make_run aaa 2026-01-01T00:00:00Z success)"
    R2="$(make_run bbb 2026-01-01T01:00:00Z success)"
    R3="$(make_run ccc 2026-01-01T02:00:00Z success)"
    R4="$(make_run ddd 2026-01-01T03:00:00Z success)"
    R5="$(make_run eee 2026-01-01T04:00:00Z failure)"
    write_payload "$R1,$R2,$R3,$R4,$R5"
    SUBJ="fix: the breaking change" export SUBJ
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 0 ]
    [[ "$output" == *"STATE:broken"* ]]
    [[ "$output" == *"LAST_GOOD:ddd"* ]]
    [[ "$output" == *"FIRST_FAILING:eee"* ]]
    [[ "$output" == *"FIRST_FAILING_SUBJECT:fix: the breaking change"* ]]
    [[ "$output" == *"COMMITS_SCANNED:5"* ]]
    [[ "$output" == *"CANCELLED_AFTER_LAST_GOOD:0"* ]]
}

@test "ok: an all-success window reports STATE:ok and FIRST_FAILING:none" {
    R1="$(make_run aaa 2026-01-01T00:00:00Z success)"
    R2="$(make_run bbb 2026-01-01T01:00:00Z success)"
    write_payload "$R1,$R2"
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 0 ]
    [[ "$output" == *"STATE:ok"* ]]
    [[ "$output" == *"LAST_GOOD:bbb"* ]]
    [[ "$output" == *"FIRST_FAILING:none"* ]]
    [[ "$output" == *"CANCELLED_AFTER_LAST_GOOD:0"* ]]
}

@test "a re-run of the same commit is deduped to its newest run" {
    R1="$(make_run aaa 2026-01-01T00:00:00Z success)"
    R2a="$(make_run bbb 2026-01-01T01:00:00Z failure)"
    R2b="$(make_run bbb 2026-01-01T02:00:00Z success)"
    R3="$(make_run ccc 2026-01-01T03:00:00Z success)"
    write_payload "$R1,$R2a,$R2b,$R3"
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 0 ]
    [[ "$output" == *"STATE:ok"* ]]
    [[ "$output" == *"LAST_GOOD:ccc"* ]]
    [[ "$output" == *"FIRST_FAILING:none"* ]]
}

@test "no success in the window: the oldest failure is first-failing" {
    R1="$(make_run aaa 2026-01-01T00:00:00Z failure)"
    R2="$(make_run bbb 2026-01-01T01:00:00Z failure)"
    write_payload "$R1,$R2"
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 0 ]
    [[ "$output" == *"STATE:broken"* ]]
    [[ "$output" == *"LAST_GOOD:none"* ]]
    [[ "$output" == *"FIRST_FAILING:aaa"* ]]
}

@test "a cancelled run after a good run never becomes the first failing" {
    # Issue #4199: `cancelled` is absent evidence. It is excluded from the
    # failure set, so a cancelled commit is never reported as
    # FIRST_FAILING — even when it predates the real failure.
    R1="$(make_run aaa 2026-01-01T00:00:00Z success)"
    R2="$(make_run bbb 2026-01-01T01:00:00Z success)"
    R3="$(make_run ccc 2026-01-01T02:00:00Z cancelled)"
    R4="$(make_run ddd 2026-01-01T03:00:00Z failure)"
    write_payload "$R1,$R2,$R3,$R4"
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 0 ]
    [[ "$output" == *"STATE:broken"* ]]
    [[ "$output" == *"LAST_GOOD:bbb"* ]]
    [[ "$output" == *"FIRST_FAILING:ddd"* ]]
    [[ "$output" == *"CANCELLED_AFTER_LAST_GOOD:1"* ]]
}

@test "only cancelled runs after the last good run: unknown, never ok" {
    # No failure and no green after the last good run: the cancelled runs
    # never verified their commits, so the window is not ok (exit 2) —
    # absent evidence, not passing evidence (issue #4199).
    R1="$(make_run aaa 2026-01-01T00:00:00Z success)"
    R2="$(make_run bbb 2026-01-01T01:00:00Z success)"
    R3="$(make_run ccc 2026-01-01T02:00:00Z success)"
    R4="$(make_run ddd 2026-01-01T03:00:00Z cancelled)"
    R5="$(make_run eee 2026-01-01T04:00:00Z cancelled)"
    write_payload "$R1,$R2,$R3,$R4,$R5"
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 2 ]
    [[ "$output" == *"STATE:unknown"* ]]
    [[ "$output" == *"LAST_GOOD:ccc"* ]]
    [[ "$output" == *"FIRST_FAILING:none"* ]]
    [[ "$output" == *"CANCELLED_AFTER_LAST_GOOD:2"* ]]
}

@test "empty runs list: STATE:unknown and exit 2 (never green)" {
    write_payload ""
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 2 ]
    [[ "$output" == *"STATE:unknown"* ]]
    [[ "$output" == *"CANCELLED_AFTER_LAST_GOOD:unknown"* ]]
}

@test "API failure: STATE:unknown and exit 2" {
    RUNS_FAIL=1 export RUNS_FAIL
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 2 ]
    [[ "$output" == *"STATE:unknown"* ]]
}

@test "missing --repo is a usage error (exit 2)" {
    run bash "$SCRIPT"
    [ "$status" -eq 2 ]
    [[ "$output" == *"--repo OWNER/REPO is required"* ]]
}

@test "non-integer --max-commits is a usage error (exit 2)" {
    run bash "$SCRIPT" --repo OWNER/REPO --max-commits abc
    [ "$status" -eq 2 ]
    [[ "$output" == *"--max-commits must be a positive integer"* ]]
}

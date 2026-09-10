#!/usr/bin/env bats
# tests/first-failing-commit.bats — issue #4108 AC3: automated
# first-failing-commit report. Stubs `gh` and drives
# scripts/first-failing-commit.sh against canned `actions/runs` payloads.
# (Suite lives at the tests/ root, so no catalog registration is needed.)

setup() {
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    mkdir -p "$TMP/bin"
    GH_LOG="$TMP/gh.log"
    : >"$GH_LOG"
    cat >"$TMP/bin/gh" <<EOF
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
    sha="\${\${*##*commits/}%% *}"
    printf '%s\n' "\${SUBJ:-subject of \$sha}"
    ;;
  *)
    printf '{}'
    ;;
esac
EOF
    chmod +x "$TMP/bin/gh"
    PATH="$TMP/bin:$PATH"
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
    printf '{"workflow_runs":[%s]}\n' "$1" >"$TMP/runs.json"
}

@test "broken: newest success and oldest failure after it are reported" {
    R1="$(make_run aaa 2026-01-01T00:00:00Z success)"
    R2="$(make_run bbb 2026-01-01T01:00:00Z success)"
    R3="$(make_run ccc 2026-01-01T02:00:00Z success)"
    R4="$(make_run ddd 2026-01-01T03:00:00Z success)"
    R5="$(make_run eee 2026-01-01T04:00:00Z failure)"
    write_payload "$R1,$R2,$R3,$R4,$R5"
    RUNS_FILE="$TMP/runs.json" export RUNS_FILE
    SUBJ="fix: the breaking change" export SUBJ
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 0 ]
    [[ "$output" == *"STATE:broken"* ]]
    [[ "$output" == *"LAST_GOOD:ddd"* ]]
    [[ "$output" == *"FIRST_FAILING:eee"* ]]
    [[ "$output" == *"FIRST_FAILING_SUBJECT:fix: the breaking change"* ]]
    [[ "$output" == *"COMMITS_SCANNED:5"* ]]
}

@test "ok: an all-success window reports STATE:ok and FIRST_FAILING:none" {
    R1="$(make_run aaa 2026-01-01T00:00:00Z success)"
    R2="$(make_run bbb 2026-01-01T01:00:00Z success)"
    write_payload "$R1,$R2"
    RUNS_FILE="$TMP/runs.json" export RUNS_FILE
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 0 ]
    [[ "$output" == *"STATE:ok"* ]]
    [[ "$output" == *"LAST_GOOD:bbb"* ]]
    [[ "$output" == *"FIRST_FAILING:none"* ]]
}

@test "a re-run of the same commit is deduped to its newest run" {
    # ccc: failed first, then re-ran green. Latest run per sha wins, so the
    # commit is not first-failing.
    R1="$(make_run aaa 2026-01-01T00:00:00Z success)"
    R2="$(make_run ccc 2026-01-01T01:00:00Z failure)"
    R3="$(make_run ccc 2026-01-01T02:00:00Z success)"
    R4="$(make_run ddd 2026-01-01T03:00:00Z failure)"
    write_payload "$R1,$R2,$R3,$R4"
    RUNS_FILE="$TMP/runs.json" export RUNS_FILE
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 0 ]
    [[ "$output" == *"STATE:broken"* ]]
    [[ "$output" == *"LAST_GOOD:ccc"* ]]
    [[ "$output" == *"FIRST_FAILING:ddd"* ]]
    [[ "$output" == *"COMMITS_SCANNED:3"* ]]
}

@test "no success in the window: the oldest failure is first-failing" {
    R1="$(make_run x1 2026-01-01T00:00:00Z failure)"
    R2="$(make_run x2 2026-01-01T01:00:00Z failure)"
    write_payload "$R1,$R2"
    RUNS_FILE="$TMP/runs.json" export RUNS_FILE
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 0 ]
    [[ "$output" == *"STATE:broken"* ]]
    [[ "$output" == *"LAST_GOOD:none"* ]]
    [[ "$output" == *"FIRST_FAILING:x1"* ]]
}

@test "a cancelled run counts as a failing conclusion" {
    R1="$(make_run aaa 2026-01-01T00:00:00Z success)"
    R2="$(make_run bbb 2026-01-01T01:00:00Z cancelled)"
    write_payload "$R1,$R2"
    RUNS_FILE="$TMP/runs.json" export RUNS_FILE
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 0 ]
    [[ "$output" == *"STATE:broken"* ]]
    [[ "$output" == *"FIRST_FAILING:bbb"* ]]
}

@test "empty runs list: STATE:unknown and exit 2 (never green)" {
    RUNS_FAIL="" export RUNS_FAIL
    run bash "$SCRIPT" --repo OWNER/REPO
    [ "$status" -eq 2 ]
    [[ "$output" == *"STATE:unknown"* ]]
    [[ "$output" == *"LAST_GOOD:none"* ]]
    [[ "$output" == *"FIRST_FAILING:none"* ]]
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

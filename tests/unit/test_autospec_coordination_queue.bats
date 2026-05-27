#!/usr/bin/env bats
# tests/unit/test_autospec_coordination_queue.bats — distributed queue planner.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/skills/autospec-run/scripts/list-ready-issues.sh"
    TEST_TMP="$(mktemp -d)"
    AUTO_JSON="$TEST_TMP/auto.json"
    ACTIVE_JSON="$TEST_TMP/active.json"
    STATES_JSON="$TEST_TMP/states.json"

    mkdir -p "$TEST_TMP/bin"
    cat > "$TEST_TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
set -eu

if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
  printf 'testorg/testrepo\n'
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  label=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --label) label="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  case "$label" in
    auto-implement) cat "${AUTOSPEC_TEST_AUTO_JSON:?}" ;;
    in-progress-by-bot) cat "${AUTOSPEC_TEST_ACTIVE_JSON:?}" ;;
    *) printf '[]\n' ;;
  esac
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  issue="$3"
  jq -r --arg issue "$issue" '.[$issue] // "OPEN"' "${AUTOSPEC_TEST_STATES_JSON:?}"
  exit 0
fi

exit 1
SH
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_TEST_AUTO_JSON="$AUTO_JSON"
    export AUTOSPEC_TEST_ACTIVE_JSON="$ACTIVE_JSON"
    export AUTOSPEC_TEST_STATES_JSON="$STATES_JSON"
    printf '[]\n' > "$ACTIVE_JSON"
    printf '{}\n' > "$STATES_JSON"
}

teardown() {
    rm -rf "$TEST_TMP"
}

body_with_path() {
    local path="$1"
    cat <<EOF
## Goal
Implement fixture.

## Implementation outline
- \`$path\`: update fixture path.

## Dependencies
None
EOF
}

@test "Depends on #1 blocks issue 2" {
    body2="$(cat <<'EOF'
## Goal
Implement dependent fixture.

## Implementation outline
- `skills/bar.sh`: update fixture path.

## Dependencies
Depends on #1
EOF
)"
    jq -n --arg body "$body2" '[{number:2,title:"dependent",body:$body,labels:[{name:"auto-implement"}]}]' > "$AUTO_JSON"
    printf '{"1":"OPEN"}\n' > "$STATES_JSON"

    run bash "$SCRIPT" --repo testorg/testrepo

    [ "$status" -eq 0 ]
    planner_output="$output"
    run bash -c "printf '%s' '$planner_output' | jq -r '.blocked[0].number'"
    [ "$output" = "2" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.ready | length'"
    [ "$output" = "0" ]
}

@test "planner groups 4 independent issues in one batch" {
    jq -n \
      --arg b1 "$(body_with_path skills/a.sh)" \
      --arg b2 "$(body_with_path skills/b.sh)" \
      --arg b3 "$(body_with_path docs/c.md)" \
      --arg b4 "$(body_with_path tests/d.bats)" \
      '[
        {number:10,title:"a",body:$b1,labels:[{name:"auto-implement"}]},
        {number:11,title:"b",body:$b2,labels:[{name:"auto-implement"}]},
        {number:12,title:"c",body:$b3,labels:[{name:"auto-implement"}]},
        {number:13,title:"d",body:$b4,labels:[{name:"auto-implement"}]}
      ]' > "$AUTO_JSON"

    run bash "$SCRIPT" --repo testorg/testrepo --batch-size 4

    [ "$status" -eq 0 ]
    run bash -c "printf '%s' '$output' | jq -r '.batch | map(.number) | join(\",\")'"
    [ "$output" = "10,11,12,13" ]
}

@test "planner excludes overlapping path skills/foo.sh" {
    jq -n --arg body "$(body_with_path skills/foo.sh)" \
      '[{number:20,title:"candidate",body:$body,labels:[{name:"auto-implement"}]}]' > "$AUTO_JSON"
    jq -n --arg body "$(body_with_path skills/foo.sh)" \
      '[{number:19,title:"active",body:$body,labels:[{name:"in-progress-by-bot"}]}]' > "$ACTIVE_JSON"

    run bash "$SCRIPT" --repo testorg/testrepo

    [ "$status" -eq 0 ]
    planner_output="$output"
    run bash -c "printf '%s' '$planner_output' | jq -r '.conflicts[0].number'"
    [ "$output" = "20" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.conflicts[0].conflicts_with'"
    [ "$output" = "19" ]
}

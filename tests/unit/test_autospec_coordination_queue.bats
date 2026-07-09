#!/usr/bin/env bats
# tests/unit/test_autospec_coordination_queue.bats — distributed queue planner.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/skills/autospec-run/scripts/list-ready-issues.sh"
    TEST_TMP="$(mktemp -d)"
    AUTO_JSON="$TEST_TMP/auto.json"
    ACTIVE_JSON="$TEST_TMP/active.json"
    STATES_JSON="$TEST_TMP/states.json"
    ISSUE_VIEWS_JSON="$TEST_TMP/views.json"

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
  if printf '%s\n' "$*" | grep -q -- '--json'; then
    jq -c --arg issue "$issue" '.[$issue] // {"state":"OPEN","body":"","labels":[]}' "${AUTOSPEC_TEST_VIEWS_JSON:?}"
    exit 0
  fi
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
    export AUTOSPEC_TEST_VIEWS_JSON="$ISSUE_VIEWS_JSON"
    printf '[]\n' > "$ACTIVE_JSON"
    printf '{}\n' > "$STATES_JSON"
    printf '{}\n' > "$ISSUE_VIEWS_JSON"
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

@test "Depends on issue #1 blocks issue 2" {
    body2="$(cat <<'EOF'
## Goal
Implement dependent fixture.

## Implementation outline
- `skills/bar.sh`: update fixture path.

## Dependencies
Depends on issue #1
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

@test "epic-labeled open dependency is non-blocking and observable" {
    body2="$(cat <<'EOF'
## Goal
Implement epic child fixture.

## Implementation outline
- `skills/child.sh`: update fixture path.

## Dependencies
Depends on issue #1
EOF
)"
    jq -n --arg body "$body2" '[{number:2,title:"child",body:$body,labels:[{name:"auto-implement"}]}]' > "$AUTO_JSON"
    jq -n '{"1":{"state":"OPEN","body":"## Goal\nTrack children.\n","labels":[{"name":"epic"}]}}' > "$ISSUE_VIEWS_JSON"
    printf '{"1":"OPEN"}\n' > "$STATES_JSON"

    run bash "$SCRIPT" --repo testorg/testrepo

    [ "$status" -eq 0 ]
    planner_output="$output"
    run bash -c "printf '%s' '$planner_output' | jq -r '.ready[0].number'"
    [ "$output" = "2" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.ready[0].non_blocking_refs[0].issue'"
    [ "$output" = "1" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.ready[0].non_blocking_refs[0].reason'"
    [ "$output" = "epic_label" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.blocked | length'"
    [ "$output" = "0" ]
}

@test "non-epic sibling dependency still blocks" {
    body2="$(cat <<'EOF'
## Goal
Implement sibling child fixture.

## Implementation outline
- `skills/child.sh`: update fixture path.

## Dependencies
Depends on issue #1
EOF
)"
    jq -n --arg body "$body2" '[{number:2,title:"child",body:$body,labels:[{name:"auto-implement"}]}]' > "$AUTO_JSON"
    jq -n '{"1":{"state":"OPEN","body":"## Goal\nImplement sibling.\n","labels":[{"name":"auto-implement"}]}}' > "$ISSUE_VIEWS_JSON"
    printf '{"1":"OPEN"}\n' > "$STATES_JSON"

    run bash "$SCRIPT" --repo testorg/testrepo

    [ "$status" -eq 0 ]
    planner_output="$output"
    run bash -c "printf '%s' '$planner_output' | jq -r '.blocked[0].number'"
    [ "$output" = "2" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.blocked[0].reason'"
    [ "$output" = "blocked_dependencies" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.blocked[0].unmet_dependencies[0]'"
    [ "$output" = "1" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.ready | length'"
    [ "$output" = "0" ]
}

@test "dependency cycle is reported distinctly instead of empty" {
    body2="$(cat <<'EOF'
## Goal
Implement cyclic A.

## Implementation outline
- `skills/a.sh`: update fixture path.

## Dependencies
Depends on issue #1
EOF
)"
    body1="$(cat <<'EOF'
## Goal
Implement cyclic B.

## Implementation outline
- `skills/b.sh`: update fixture path.

## Dependencies
Depends on issue #2
EOF
)"
    jq -n --arg body "$body2" '[{number:2,title:"cyclic-a",body:$body,labels:[{name:"auto-implement"}]}]' > "$AUTO_JSON"
    jq -n --arg body "$body1" '{"1":{"state":"OPEN","body":$body,"labels":[{"name":"auto-implement"}]}}' > "$ISSUE_VIEWS_JSON"
    printf '{"1":"OPEN"}\n' > "$STATES_JSON"

    run bash "$SCRIPT" --repo testorg/testrepo

    [ "$status" -eq 0 ]
    planner_output="$output"
    run bash -c "printf '%s' '$planner_output' | jq -r '.blocked[0].number'"
    [ "$output" = "2" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.blocked[0].reason'"
    [ "$output" = "blocked_cycle" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.blocked[0].cycle_dependencies[0]'"
    [ "$output" = "1" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.blocked | length'"
    [ "$output" = "1" ]
}

@test "children tracker back-edge is non-blocking and marked as a cycle reference" {
    body2="$(cat <<'EOF'
## Goal
Implement tracker child fixture.

## Implementation outline
- `skills/child.sh`: update fixture path.

## Dependencies
Depends on issue #1
EOF
)"
    tracker="$(cat <<'EOF'
## Goal
Track children.

## Children
- [ ] #2 implement child
EOF
)"
    jq -n --arg body "$body2" '[{number:2,title:"child",body:$body,labels:[{name:"auto-implement"}]}]' > "$AUTO_JSON"
    jq -n --arg body "$tracker" '{"1":{"state":"OPEN","body":$body,"labels":[]}}' > "$ISSUE_VIEWS_JSON"
    printf '{"1":"OPEN"}\n' > "$STATES_JSON"

    run bash "$SCRIPT" --repo testorg/testrepo

    [ "$status" -eq 0 ]
    planner_output="$output"
    run bash -c "printf '%s' '$planner_output' | jq -r '.ready[0].number'"
    [ "$output" = "2" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.ready[0].non_blocking_refs[0].reason'"
    [ "$output" = "children_back_edge" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.ready[0].non_blocking_refs[0].cycle'"
    [ "$output" = "true" ]
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

@test "planner returns empty batch when repo worker cap is reached" {
    jq -n \
      --arg b1 "$(body_with_path skills/a.sh)" \
      --arg b2 "$(body_with_path skills/b.sh)" \
      '[
        {number:10,title:"a",body:$b1,labels:[{name:"auto-implement"}]},
        {number:11,title:"b",body:$b2,labels:[{name:"auto-implement"}]}
      ]' > "$AUTO_JSON"
    jq -n --arg body "$(body_with_path skills/active.sh)" \
      '[{number:9,title:"active",body:$body,labels:[{name:"in-progress-by-bot"}]}]' > "$ACTIVE_JSON"

    AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS=1 run bash "$SCRIPT" --repo testorg/testrepo --batch-size 2

    [ "$status" -eq 0 ]
    planner_output="$output"
    run bash -c "printf '%s' '$planner_output' | jq -r '.ready | length'"
    [ "$output" = "2" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.batch | length'"
    [ "$output" = "0" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.worker_cap.reached'"
    [ "$output" = "true" ]
}

@test "planner limits batch to remaining repo worker capacity" {
    jq -n \
      --arg b1 "$(body_with_path skills/a.sh)" \
      --arg b2 "$(body_with_path skills/b.sh)" \
      --arg b3 "$(body_with_path skills/c.sh)" \
      '[
        {number:10,title:"a",body:$b1,labels:[{name:"auto-implement"}]},
        {number:11,title:"b",body:$b2,labels:[{name:"auto-implement"}]},
        {number:12,title:"c",body:$b3,labels:[{name:"auto-implement"}]}
      ]' > "$AUTO_JSON"
    jq -n --arg body "$(body_with_path skills/active.sh)" \
      '[{number:9,title:"active",body:$body,labels:[{name:"in-progress-by-bot"}]}]' > "$ACTIVE_JSON"

    AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS=2 run bash "$SCRIPT" --repo testorg/testrepo --batch-size 3

    [ "$status" -eq 0 ]
    planner_output="$output"
    run bash -c "printf '%s' '$planner_output' | jq -r '.worker_cap.remaining'"
    [ "$output" = "1" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.batch | map(.number) | join(\",\")'"
    [ "$output" = "10" ]
}

@test "planner explains serialized labels and keeps deep work one-at-a-time" {
    jq -n \
      --arg deep "$(body_with_path skills/deep.sh)" \
      --arg safe "$(body_with_path skills/safe.sh)" \
      '[
        {number:40,title:"deep",body:$deep,labels:[{name:"auto-implement"},{name:"reasoning:deep"}]},
        {number:41,title:"safe",body:$safe,labels:[{name:"auto-implement"}]}
      ]' > "$AUTO_JSON"

    AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS=3 run bash "$SCRIPT" --repo testorg/testrepo --batch-size 3

    [ "$status" -eq 0 ]
    planner_output="$output"
    run bash -c "printf '%s' '$planner_output' | jq -r '.ready[] | select(.number == 40).parallel_safe'"
    [ "$output" = "false" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.ready[] | select(.number == 40).serialization_reasons[0]'"
    [ "$output" = "reasoning:deep" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.ready[] | select(.number == 41).parallel_safe'"
    [ "$output" = "true" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.batch | map(.number) | join(\",\")'"
    [ "$output" = "40" ]
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

@test "planner batch excludes ready issues that overlap each other" {
    jq -n \
      --arg b1 "$(body_with_path skills/shared.sh)" \
      --arg b2 "$(body_with_path skills/shared.sh)" \
      --arg b3 "$(body_with_path docs/independent.md)" \
      '[
        {number:30,title:"shared-a",body:$b1,labels:[{name:"auto-implement"}]},
        {number:31,title:"shared-b",body:$b2,labels:[{name:"auto-implement"}]},
        {number:32,title:"independent",body:$b3,labels:[{name:"auto-implement"}]}
      ]' > "$AUTO_JSON"

    run bash "$SCRIPT" --repo testorg/testrepo --batch-size 3

    [ "$status" -eq 0 ]
    planner_output="$output"
    run bash -c "printf '%s' '$planner_output' | jq -r '.batch | map(.number) | join(\",\")'"
    [ "$output" = "30,32" ]
    run bash -c "printf '%s' '$planner_output' | jq -r '.conflicts[] | select(.number == 31).conflicts_with'"
    [ "$output" = "30" ]
}

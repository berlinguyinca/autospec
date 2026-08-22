#!/usr/bin/env bats
# tests/unit/test_autospec_run_state.bats — Rust GitHub run-state command.
if [ -z "${BATS_VERSION:-}" ]; then
    exec bats "$0" "$@"
fi

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    AUTOSPEC="$REPO_ROOT/target/debug/autospec"
    TEST_TMP="$(mktemp -d)"
    COMMENTS="$TEST_TMP/comments.json"
    CALLS="$TEST_TMP/calls.log"
    PRS="$TEST_TMP/prs.json"
    printf '[]\n' > "$COMMENTS"
    printf '[]\n' > "$PRS"

    mkdir -p "$TEST_TMP/bin"
    cat > "$TEST_TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
set -eu

comments="${AUTOSPEC_TEST_COMMENTS:?}"
calls="${AUTOSPEC_TEST_CALLS:?}"
printf '%s\n' "$*" >> "$calls"

if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
  printf 'testorg/testrepo\n'
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then
  body=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --body-file) body="$(cat "$2")"; shift 2 ;;
      --body) body="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  jq --arg body "$body" '. + [{id: 1, body: $body, updated_at:"2026-07-14T00:00:00Z"}]' "$comments" > "$comments.tmp"
  mv "$comments.tmp" "$comments"
  exit 0
fi


if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  if [ -f "${AUTOSPEC_TEST_PRS:-}" ]; then
    jq '[.[] | {number,body}]' "$AUTOSPEC_TEST_PRS"
  else
    printf '[]\n'
  fi
  exit 0
fi

if [ "$1" = "api" ]; then
  method=""
  body=""
  url="$2"
  shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -X) method="$2"; shift 2 ;;
      -F)
        case "$2" in body=@*) body="$(cat "${2#body=@}")" ;; esac
        shift 2
        ;;
      -f)
        case "$2" in body=*) body="${2#body=}" ;; esac
        shift 2
        ;;
      *) shift ;;
    esac
  done
  id="${url##*/}"
  if [[ "$url" = repos/*/issues/42/comments ]]; then
    cat "$comments"
    exit 0
  fi
  case "$method" in
    PATCH)
      if [ -n "${AUTOSPEC_TEST_FAIL_PATCH_ONCE:-}" ] && [ ! -f "${AUTOSPEC_TEST_PATCH_FAIL_MARKER:?}" ]; then
        touch "$AUTOSPEC_TEST_PATCH_FAIL_MARKER"
        exit 1
      fi
      jq --argjson id "$id" --arg body "$body" \
        'map(if .id == $id then .body = $body | .updated_at = "2026-07-14T00:00:00Z" else . end)' "$comments" > "$comments.tmp"
      mv "$comments.tmp" "$comments"
      ;;
    DELETE)
      jq --argjson id "$id" 'map(select(.id != $id))' "$comments" > "$comments.tmp"
      mv "$comments.tmp" "$comments"
      ;;
  esac
  exit 0
fi

exit 1
SH
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_TEST_COMMENTS="$COMMENTS"
    export AUTOSPEC_TEST_CALLS="$CALLS"
    export AUTOSPEC_TEST_PRS="$PRS"
    export AUTOSPEC_TEST_FAIL_PATCH_ONCE=""
    export AUTOSPEC_TEST_PATCH_FAIL_MARKER="$TEST_TMP/patch-failed-once"
    export AUTOSPEC_GH_API_RETRY_SLEEP=0
}

teardown() {
    rm -rf "$TEST_TMP"
}

run_state() {
    "$AUTOSPEC" claim state "$@"
}

@test "claim state supports read, upsert, and clear commands" {
    run run_state upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state claimed --step claimed
    [ "$status" -eq 0 ]
    [[ "$output" == *'"state":"claimed"'* ]]

    run run_state read --issue 42 --repo testorg/testrepo
    [ "$status" -eq 0 ]
    [[ "$output" == *'"worker_id":"worker-a"'* ]]

    run run_state clear --issue 42 --repo testorg/testrepo
    [ "$status" -eq 0 ]
    run run_state read --issue 42 --repo testorg/testrepo
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "repeated upsert keeps 1 marked state comment" {
    run_state upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state claimed --step claimed >/dev/null
    run_state upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state worktree_ready --step worktree_ready >/dev/null

    run jq '[.[].body | select(contains("autospec-run-state:begin"))] | length' "$COMMENTS"
    [ "$status" -eq 0 ]
    [ "$output" = "1" ]
    run run_state read --issue 42 --repo testorg/testrepo
    [[ "$output" == *'"state":"worktree_ready"'* ]]
}

@test "invalid schema 0 is replaced" {
    cat > "$COMMENTS" <<'JSON'
[
  {
    "id": 7,
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":0,\"issue\":42,\"repo\":\"testorg/testrepo\"}\n<!-- autospec-run-state:end -->"
  }
]
JSON

    run run_state upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state claimed --step claimed
    [ "$status" -eq 0 ]

    run jq -r '.[0].id' "$COMMENTS"
    [ "$output" = "7" ]
    run jq -r '.[0].body | contains("\"schema\":1")' "$COMMENTS"
    [ "$output" = "true" ]
    run jq '[.[].body | select(contains("autospec-run-state:begin"))] | length' "$COMMENTS"
    [ "$output" = "1" ]
}

@test "claim state ignores state comments for another repo" {
    run_state upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state claimed --step claimed >/dev/null

    run run_state read --issue 42 --repo otherorg/otherrepo

    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "upsert collapses duplicate marked state comments" {
    cat > "$COMMENTS" <<'JSON'
[
  {
    "id": 7,
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":1,\"issue\":42,\"repo\":\"testorg/testrepo\",\"worker_id\":\"old-a\",\"state\":\"claimed\",\"claimed_at\":\"2026-01-01T00:00:00Z\"}\n<!-- autospec-run-state:end -->"
  },
  {
    "id": 8,
    "body": "<!-- autospec-run-state:begin -->\n{\"schema\":1,\"issue\":42,\"repo\":\"testorg/testrepo\",\"worker_id\":\"old-b\",\"state\":\"claimed\",\"claimed_at\":\"2026-01-01T00:00:00Z\"}\n<!-- autospec-run-state:end -->"
  }
]
JSON

    run run_state upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state worktree_ready

    [ "$status" -eq 0 ]
    run jq '[.[].body | select(contains("autospec-run-state:begin"))] | length' "$COMMENTS"
    [ "$output" = "1" ]
    run jq -r '.[0].id' "$COMMENTS"
    [ "$output" = "7" ]
    run run_state read --issue 42 --repo testorg/testrepo
    [[ "$output" == *'"worker_id":"worker-a"'* ]]
}

@test "upsert retries transient REST patch failure" {
    run_state upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state claimed >/dev/null
    export AUTOSPEC_TEST_FAIL_PATCH_ONCE=1

    run run_state upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state worktree_ready

    [ "$status" -eq 0 ]
    [ -f "$AUTOSPEC_TEST_PATCH_FAIL_MARKER" ]
    run run_state read --issue 42 --repo testorg/testrepo
    [[ "$output" == *'"state":"worktree_ready"'* ]]
}

@test "phase4 pr creation marks heartbeat and run-state with the PR number" {
    prompt="$REPO_ROOT/skills/autospec-run/prompts/phase4-implementer.md"
    run grep -Fq 'pr_number="$(gh pr view "$pr_url" --json number --jq .number)"' "$prompt"
    [ "$status" -eq 0 ]
    run grep -Fq '[ -n "$pr_number" ] && [ "$pr_number" != "null" ]' "$prompt"
    [ "$status" -eq 0 ]
    heartbeat_cmd='heartbeat-write.sh" --issue <ISSUE> --repo <REPO> --branch "$branch_name" --step pr_created --pr "$pr_number" --worker-id "$CLAIM_WORKER_ID" --claim-id "$CLAIM_ID" --session-id "$WAIT_TARGET_SESSION_ID" || exit 1'
    run_state_cmd='autospec claim state upsert --issue <ISSUE> --repo <REPO> --worker-id "$worker_id" --state claimed --step pr_created --branch "$branch_name" --pr "$pr_number" || exit 1'
    run grep -Fq "$heartbeat_cmd" "$prompt"
    [ "$status" -eq 0 ]
    run grep -Fq "$run_state_cmd" "$prompt"
    [ "$status" -eq 0 ]
    pr_number_line="$(grep -nF 'pr_number="$(gh pr view "$pr_url" --json number --jq .number)"' "$prompt" | cut -d: -f1)"
    heartbeat_line="$(grep -nF "$heartbeat_cmd" "$prompt" | cut -d: -f1)"
    run_state_line="$(grep -nF "$run_state_cmd" "$prompt" | cut -d: -f1)"
    handoff_line="$(grep -nF 'any later handoff' "$prompt" | cut -d: -f1)"
    [ "$pr_number_line" -lt "$heartbeat_line" ]
    [ "$heartbeat_line" -lt "$run_state_line" ]
    [ "$run_state_line" -lt "$handoff_line" ]

    heartbeat_dir="$TEST_TMP/heartbeats"
    export AUTOSPEC_HEARTBEAT_DIR="$heartbeat_dir"
    heartbeat_write="$REPO_ROOT/skills/autospec-run/scripts/heartbeat-write.sh"

    bash "$heartbeat_write" --issue 42 --repo testorg/testrepo --branch feat/test --step claimed
    run_state upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state claimed --step claimed --branch feat/test >/dev/null

    pr_number=1858
    bash "$heartbeat_write" --issue 42 --repo testorg/testrepo --branch feat/test --step pr_created --pr "$pr_number"
    run_state upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state claimed --step pr_created --branch feat/test --pr "$pr_number" >/dev/null

    state="$(run_state read --issue 42 --repo testorg/testrepo)"
    [ "$(printf '%s' "$state" | jq -r '.state')" = "pr_created" ]
    [ "$(printf '%s' "$state" | jq -r '.pr')" = "$pr_number" ]

    heartbeat_file="$heartbeat_dir/testorg__testrepo/42.json"
    [ -f "$heartbeat_file" ]
    [ "$(jq -r '.step' "$heartbeat_file")" = "pr_created" ]
    [ "$(jq -r '.pr' "$heartbeat_file")" = "$pr_number" ]
}


@test "reconcile-linked-pr records open PR with one Closeout report before handoff relabel" {
    run_state upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state claimed --step claimed >/dev/null
    jq -n '[{number:1857,title:"fix: recover claim",url:"https://github.example/pr/1857",body:"Closes #42

## Closeout report

**Result** shipped."}]' > "$PRS"

    run run_state reconcile-linked-pr --issue 42 --repo testorg/testrepo

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.reconciled')" = "true" ]
    [ "$(printf '%s' "$output" | jq -r '.pr')" = "1857" ]

    state="$(run_state read --issue 42 --repo testorg/testrepo)"
    [ "$(printf '%s' "$state" | jq -r '.pr')" = "1857" ]
    [ "$(printf '%s' "$state" | jq -r '.step')" = "post_pr_handoff_failed" ]
    [ "$(printf '%s' "$state" | jq -r '.updated_at | length > 0')" = "true" ]

    run jq -r '[.[].body | select(contains("autospec-linked-pr-run-state-reconcile:pr:1857"))] | length' "$COMMENTS"
    [ "$output" = "1" ]
    run jq -r '[.[].body | select(contains("Resume post-PR handoff from PR #1857"))] | length' "$COMMENTS"
    [ "$output" = "1" ]
}

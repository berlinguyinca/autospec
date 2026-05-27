#!/usr/bin/env bats
# tests/unit/test_autospec_run_state.bats — GitHub run-state comment helper.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/skills/autospec-run/scripts/run-state.sh"
    TEST_TMP="$(mktemp -d)"
    COMMENTS="$TEST_TMP/comments.json"
    CALLS="$TEST_TMP/calls.log"
    printf '[]\n' > "$COMMENTS"

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

if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  cat "$comments"
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then
  body_file=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --body-file) body_file="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  jq --arg body "$(cat "$body_file")" '. + [{id: 1, body: $body}]' "$comments" > "$comments.tmp"
  mv "$comments.tmp" "$comments"
  exit 0
fi

if [ "$1" = "api" ]; then
  method=""
  body_file=""
  url="$2"
  shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -X) method="$2"; shift 2 ;;
      -F)
        case "$2" in body=@*) body_file="${2#body=@}" ;; esac
        shift 2
        ;;
      *) shift ;;
    esac
  done
  id="${url##*/}"
  case "$method" in
    PATCH)
      jq --argjson id "$id" --arg body "$(cat "$body_file")" \
        'map(if .id == $id then .body = $body else . end)' "$comments" > "$comments.tmp"
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
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "run-state.sh supports read, upsert, and clear commands" {
    run bash "$SCRIPT" upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state claimed --step claimed
    [ "$status" -eq 0 ]
    [[ "$output" == *'"state": "claimed"'* ]]

    run bash "$SCRIPT" read --issue 42 --repo testorg/testrepo
    [ "$status" -eq 0 ]
    [[ "$output" == *'"worker_id": "worker-a"'* ]]

    run bash "$SCRIPT" clear --issue 42 --repo testorg/testrepo
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" read --issue 42 --repo testorg/testrepo
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "repeated upsert keeps 1 marked state comment" {
    bash "$SCRIPT" upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state claimed --step claimed >/dev/null
    bash "$SCRIPT" upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state worktree_ready --step worktree_ready >/dev/null

    run jq '[.[].body | select(contains("autospec-run-state:begin"))] | length' "$COMMENTS"
    [ "$status" -eq 0 ]
    [ "$output" = "1" ]
    run bash "$SCRIPT" read --issue 42 --repo testorg/testrepo
    [[ "$output" == *'"state": "worktree_ready"'* ]]
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

    run bash "$SCRIPT" upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state claimed --step claimed
    [ "$status" -eq 0 ]

    run jq -r '.[0].id' "$COMMENTS"
    [ "$output" = "7" ]
    run jq -r '.[0].body | contains("\"schema\": 1")' "$COMMENTS"
    [ "$output" = "true" ]
    run jq '[.[].body | select(contains("autospec-run-state:begin"))] | length' "$COMMENTS"
    [ "$output" = "1" ]
}

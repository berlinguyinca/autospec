#!/usr/bin/env bats
# tests/unit/test_autospec_linked_pr_reconcile.bats — queue scan reconciles
# claimed issues whose linked PR exists but run-state still lacks `.pr`.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LIST_READY="$REPO_ROOT/skills/autospec-run/scripts/list-ready-issues.sh"
    RUN_STATE="$REPO_ROOT/skills/autospec-run/scripts/run-state.sh"
    TEST_TMP="$(mktemp -d)"
    AUTO_JSON="$TEST_TMP/auto.json"
    ACTIVE_JSON="$TEST_TMP/active.json"
    COMMENTS="$TEST_TMP/comments.json"
    PRS="$TEST_TMP/prs.json"
    CALLS="$TEST_TMP/calls.log"
    mkdir -p "$TEST_TMP/bin"
    printf '[]\n' > "$AUTO_JSON"
    jq -n '[{number:42,title:"claimed",body:"",labels:[{name:"in-progress-by-bot"}]}]' > "$ACTIVE_JSON"
    printf '[]\n' > "$COMMENTS"
    jq -n '[{number:1857,title:"fix: claimed",url:"https://github.example/pr/1857",body:"Closes #42\n\n## Closeout report\n\n**Result** shipped."}]' > "$PRS"

    cat > "$TEST_TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "${AUTOSPEC_TEST_CALLS:?}"
if [ "$1" = "repo" ] && [ "$2" = "view" ]; then printf 'testorg/testrepo\n'; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  label=""
  while [ "$#" -gt 0 ]; do
    case "$1" in --label) label="$2"; shift 2 ;; *) shift ;; esac
  done
  case "$label" in
    auto-implement) cat "${AUTOSPEC_TEST_AUTO_JSON:?}" ;;
    in-progress-by-bot) cat "${AUTOSPEC_TEST_ACTIVE_JSON:?}" ;;
    *) printf '[]\n' ;;
  esac
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then cat "${AUTOSPEC_TEST_PRS:?}"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then
  body_file=""
  while [ "$#" -gt 0 ]; do
    case "$1" in --body-file) body_file="$2"; shift 2 ;; *) shift ;; esac
  done
  next_id="$(jq '([.[].id] | max // 0) + 1' "${AUTOSPEC_TEST_COMMENTS:?}")"
  jq --argjson id "$next_id" --arg body "$(cat "$body_file")" '. + [{id:$id, body:$body}]' \
    "${AUTOSPEC_TEST_COMMENTS:?}" > "${AUTOSPEC_TEST_COMMENTS:?}.tmp"
  mv "${AUTOSPEC_TEST_COMMENTS:?}.tmp" "${AUTOSPEC_TEST_COMMENTS:?}"
  exit 0
fi
if [ "$1" = "api" ]; then
  method=""; body_file=""; url="$2"; shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -X) method="$2"; shift 2 ;;
      -F) case "$2" in body=@*) body_file="${2#body=@}" ;; esac; shift 2 ;;
      *) shift ;;
    esac
  done
  id="${url##*/}"
  if [ "$url" = "repos/testorg/testrepo/issues/42/comments" ]; then cat "${AUTOSPEC_TEST_COMMENTS:?}"; exit 0; fi
  case "$method" in
    PATCH)
      jq --argjson id "$id" --arg body "$(cat "$body_file")" 'map(if .id == $id then .body = $body else . end)' \
        "${AUTOSPEC_TEST_COMMENTS:?}" > "${AUTOSPEC_TEST_COMMENTS:?}.tmp"
      mv "${AUTOSPEC_TEST_COMMENTS:?}.tmp" "${AUTOSPEC_TEST_COMMENTS:?}" ;;
    DELETE)
      jq --argjson id "$id" 'map(select(.id != $id))' "${AUTOSPEC_TEST_COMMENTS:?}" > "${AUTOSPEC_TEST_COMMENTS:?}.tmp"
      mv "${AUTOSPEC_TEST_COMMENTS:?}.tmp" "${AUTOSPEC_TEST_COMMENTS:?}" ;;
  esac
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then printf '{"state":"OPEN","body":"","labels":[]}\n'; exit 0; fi
exit 1
SH
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_TEST_AUTO_JSON="$AUTO_JSON"
    export AUTOSPEC_TEST_ACTIVE_JSON="$ACTIVE_JSON"
    export AUTOSPEC_TEST_COMMENTS="$COMMENTS"
    export AUTOSPEC_TEST_PRS="$PRS"
    export AUTOSPEC_TEST_CALLS="$CALLS"
    export AUTOSPEC_CONFIG_FILE="$TEST_TMP/missing-autospec.yml"
    export AUTOSPEC_GH_API_RETRY_SLEEP=0

    bash "$RUN_STATE" upsert --issue 42 --repo testorg/testrepo --worker-id worker-a --state claimed --step claimed >/dev/null
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "list-ready reconciles active claimed issue with linked PR and one Closeout report" {
    run bash "$LIST_READY" --repo testorg/testrepo --batch-size 1

    [ "$status" -eq 0 ]
    state="$(bash "$RUN_STATE" read --issue 42 --repo testorg/testrepo)"
    [ "$(printf '%s' "$state" | jq -r '.pr')" = "1857" ]
    [ "$(printf '%s' "$state" | jq -r '.step')" = "post_pr_handoff_failed" ]
    run jq -r '[.[].body | select(contains("Resume post-PR handoff from PR #1857"))] | length' "$COMMENTS"
    [ "$output" = "1" ]
    ! grep -q 'issue edit' "$CALLS"
}

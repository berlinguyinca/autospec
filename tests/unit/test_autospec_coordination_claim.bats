#!/usr/bin/env bats
# tests/unit/test_autospec_coordination_claim.bats — Rust claim/release commands.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    AUTOSPEC="$REPO_ROOT/target/debug/autospec"
    TEST_TMP="$(mktemp -d)"
    LABELS="$TEST_TMP/labels.txt"
    COMMENTS="$TEST_TMP/comments.json"
    CALLS="$TEST_TMP/calls.log"
    printf 'auto-implement\nctx:32k\nsafety:reviewed\n' > "$LABELS"
    printf '[]\n' > "$COMMENTS"

    mkdir -p "$TEST_TMP/bin"
    cat > "$TEST_TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
set -eu

labels="${AUTOSPEC_TEST_LABELS:?}"
comments="${AUTOSPEC_TEST_COMMENTS:?}"
calls="${AUTOSPEC_TEST_CALLS:?}"
printf '%s\n' "$*" >> "$calls"

if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
  printf 'testorg/testrepo\n'
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  wants_comments=0
  if printf '%s\n' "$*" | grep -q -- '--json comments'; then
    wants_comments=1
  fi
  jq_expr=""
  shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --jq) jq_expr="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  if [ "$wants_comments" -eq 1 ]; then
    cat "$comments"
  else
    issue_json="$(jq -Rn '
      [inputs | select(length > 0) | {name:.}] as $labels |
      {
        labels:$labels,
        body:"## Safety review\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n",
        title:"Autospec test issue",
        author:{login:"autospec-test"}
      }
    ' < "$labels")"
    if [ -n "$jq_expr" ]; then
      printf '%s\n' "$issue_json" | jq -r "$jq_expr"
    else
      printf '%s\n' "$issue_json"
    fi
  fi
  exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
  shift 2
  add=""
  remove=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --add-label) add="$2"; shift 2 ;;
      --remove-label) remove="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  if [ -n "$remove" ]; then
    tr ',' '\n' <<<"$remove" | while IFS= read -r label; do
      [ -n "$label" ] || continue
      grep -Fxv "$label" "$labels" > "$labels.tmp" && mv "$labels.tmp" "$labels"
    done
  fi
  if [ -n "$add" ]; then
    tr ',' '\n' <<<"$add" | while IFS= read -r label; do
      [ -n "$label" ] || continue
      grep -Fx "$label" "$labels" >/dev/null 2>&1 || printf '%s\n' "$label" >> "$labels"
    done
  fi
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
  if [ -n "${AUTOSPEC_TEST_FORCE_OWNER:-}" ]; then
    body="$(printf '%s\n' "$body" | sed "s/\"worker_id\":[[:space:]]*\"[^\"]*\"/\"worker_id\":\"${AUTOSPEC_TEST_FORCE_OWNER}\"/")"
  fi
  jq --arg body "$body" '. + [{id: 1, body: $body, updated_at:"2026-07-14T00:00:00Z"}]' "$comments" > "$comments.tmp"
  mv "$comments.tmp" "$comments"
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
      -F) case "$2" in body=@*) body="$(cat "${2#body=@}")" ;; esac; shift 2 ;;
      -f) case "$2" in body=*) body="${2#body=}" ;; esac; shift 2 ;;
      *) shift ;;
    esac
  done
  id="${url##*/}"
  if [ "$url" = "repos/testorg/testrepo/issues/42/comments" ]; then
    cat "$comments"
    exit 0
  fi
  if [ "$method" = "PATCH" ]; then
    if [ -n "${AUTOSPEC_TEST_FORCE_OWNER:-}" ]; then
      body="$(printf '%s\n' "$body" | sed "s/\"worker_id\":[[:space:]]*\"[^\"]*\"/\"worker_id\":\"${AUTOSPEC_TEST_FORCE_OWNER}\"/")"
    fi
    jq --argjson id "$id" --arg body "$body" \
      'map(if .id == $id then .body = $body | .updated_at = "2026-07-14T00:00:00Z" else . end)' "$comments" > "$comments.tmp"
    mv "$comments.tmp" "$comments"
  fi
  exit 0
fi

exit 1
SH
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    export AUTOSPEC_TEST_LABELS="$LABELS"
    export AUTOSPEC_TEST_COMMENTS="$COMMENTS"
    export AUTOSPEC_TEST_CALLS="$CALLS"
    export AUTOSPEC_CONFIG_FILE="$TEST_TMP/missing-autospec.yml"
    export AUTOSPEC_TEST_FORCE_OWNER=""
}

teardown() {
    rm -rf "$TEST_TMP"
}

claim_acquire() {
    "$AUTOSPEC" claim acquire "$@"
}

claim_release() {
    "$AUTOSPEC" claim release "$@"
}

@test "only 1 of 2 workers claims issue 42" {
    run claim_acquire --issue 42 --repo testorg/testrepo --worker-id worker-a
    [ "$status" -eq 0 ]
    [[ "$output" == *'"claimed":true'* ]]

    run claim_acquire --issue 42 --repo testorg/testrepo --worker-id worker-b
    [ "$status" -eq 2 ]
    [[ "$output" == *'"claimed":false'* ]]

    run bash -c "grep -c '^in-progress-by-bot$' '$LABELS'"
    [ "$output" = "1" ]
}

@test "release restores auto-implement" {
    claim_acquire --issue 42 --repo testorg/testrepo --worker-id worker-a >/dev/null

    run claim_release --issue 42 --repo testorg/testrepo --worker-id worker-a

    [ "$status" -eq 0 ]
    grep -Fx auto-implement "$LABELS"
    ! grep -Fx in-progress-by-bot "$LABELS"
    run jq -r '.[0].body | contains("\"state\":\"released\"")' "$COMMENTS"
    [ "$output" = "true" ]
}

@test "release merged removes in-progress label without restoring auto-implement" {
    claim_acquire --issue 42 --repo testorg/testrepo --worker-id worker-a >/dev/null

    run claim_release --issue 42 --repo testorg/testrepo --worker-id worker-a --state merged --pr 99

    [ "$status" -eq 0 ]
    ! grep -Fx in-progress-by-bot "$LABELS"
    ! grep -Fx auto-implement "$LABELS"
    run jq -r '.[0].body | contains("\"state\":\"merged\"") and contains("\"pr\":\"99\"")' "$COMMENTS"
    [ "$output" = "true" ]
}

@test "claim acquire exits 0 for claimed and 2 for already claimed" {
    run claim_acquire --issue 42 --repo testorg/testrepo --worker-id worker-a
    [ "$status" -eq 0 ]

    run claim_acquire --issue 42 --repo testorg/testrepo --worker-id worker-b
    [ "$status" -eq 2 ]
}

@test "claim records explicit cluster lease seconds in run-state" {
    AUTOSPEC_CLAIM_LEASE_SECONDS=7200 run claim_acquire --issue 42 --repo testorg/testrepo --worker-id worker-a

    [ "$status" -eq 0 ]
    run bash -c "jq -r '.[0].body' '$COMMENTS' | awk '/autospec-run-state:begin/{inside=1; next} /autospec-run-state:end/{inside=0} inside{print}' | jq -r '.ttl_seconds'"
    [ "$output" = "7200" ]
}

@test "claim acquire exits 2 when run-state ownership is lost" {
    AUTOSPEC_TEST_FORCE_OWNER=worker-b run claim_acquire --issue 42 --repo testorg/testrepo --worker-id worker-a

    [ "$status" -eq 2 ]
    [[ "$output" == *'"claimed": false'* ]]
    [[ "$output" == *'"reason":"claim_lost"'* ]]
}

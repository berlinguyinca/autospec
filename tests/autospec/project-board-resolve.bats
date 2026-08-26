#!/usr/bin/env bats
# Coverage for scripts/project-board-resolve.sh — pure board reader.

setup() {
  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-resolve.sh"
  FIX="${BATS_TEST_DIRNAME}/../fixtures/project-board"
}
teardown() { rm -rf "$TMP"; }

@test "parses an org project URL into identity" {
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit identity
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.owner == "InferWeave" and .kind == "org" and .number == 2'
}

@test "parses a user project URL into identity" {
  run bash "$SCRIPT" --url https://github.com/users/berlinguyinca/projects/7 --emit identity
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.owner == "berlinguyinca" and .kind == "user" and .number == 7'
}

@test "rejects a non-project URL with exit 2" {
  run bash "$SCRIPT" --url https://github.com/InferWeave/inferweave-workbench/issues/1 --emit identity
  [ "$status" -eq 2 ]
}

@test "rejects a trailing-garbage project URL with exit 2" {
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2x --emit identity
  [ "$status" -eq 2 ]
}

@test "--url with no value exits 2" {
  run bash "$SCRIPT" --url --emit identity
  [ "$status" -eq 2 ]
}

@test "--emit with no value exits 2" {
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit
  [ "$status" -eq 2 ]
}

@test "accepts org project URL with /views/N suffix" {
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2/views/1 --emit identity
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.owner == "InferWeave" and .kind == "org" and .number == 2'
}

@test "accepts user project URL with /views/N suffix" {
  run bash "$SCRIPT" --url https://github.com/users/berlinguyinca/projects/7/views/3 --emit identity
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.owner == "berlinguyinca" and .kind == "user" and .number == 7'
}

@test "rejects project URL with non-views trailing path" {
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2/junk --emit identity
  [ "$status" -eq 2 ]
}

@test "normalizes leading-zero project number" {
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/02 --emit identity
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.owner == "InferWeave" and .kind == "org" and .number == 2'
}

stub_gh() {
  cat > "$TMP/bin/gh" <<SH
#!/usr/bin/env bash
case "\$*" in
  *"project field-list"*) cat "$1" ;;
  *"project item-list"*)  cat "$2" ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
}

@test "plan exposes the AutoSpec state field id and its option ids" {
  stub_gh "$FIX/p2-fields.json" "$FIX/p2-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.fields.autospec_state.id | startswith("PVTSSF_")'
  echo "$output" | jq -e '.fields.autospec_state.options | has("Ready") and has("Done")'
}

@test "plan lists every distinct repo on a multi-repo board" {
  stub_gh "$FIX/p1-fields.json" "$FIX/p1-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit repos
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq 'length')" -eq 6 ]
  echo "$output" | jq -e 'index("InferWeave/inferweave-protocol") != null'
}

@test "plan carries item id, repo, number, labels and body" {
  stub_gh "$FIX/p2-fields.json" "$FIX/p2-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$(echo "$output" | jq '.items | length')" -eq 80 ]
  echo "$output" | jq -e '.items[0] | has("item_id") and has("repo") and has("number") and has("labels") and has("body")'
  echo "$output" | jq -e '.items[] | select(.number==2) | .repo == "InferWeave/inferweave-workbench"'
}

@test "a truncated read exits 4 and emits no plan" {
  # A full page (limit reached exactly) is indistinguishable from truncation → fail closed.
  jq '{items: .items[0:2]}' "$FIX/p2-items.json" > "$TMP/two.json"
  stub_gh "$FIX/p2-fields.json" "$TMP/two.json"
  AUTOSPEC_PROJECT_BOARD_LIMIT=2 run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 4 ]
}

@test "missing gh exits 3" {
  # A PATH containing only /usr/bin:/bin keeps bash and jq resolvable (both
  # live there) while dropping gh, which lives elsewhere (e.g. /opt/homebrew/bin).
  # Fully emptying PATH (the brief's original approach) makes `env` unable to
  # find bash itself, so `run` observes exit 127 from env, not exit 3 from the
  # script — verified directly against this shell before changing the test.
  run env PATH="/usr/bin:/bin" bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 3 ]
}

@test "the state field resolves via the p1 candidate name when AutoSpec state is absent" {
  stub_gh "$FIX/p1-fields.json" "$FIX/p1-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit plan
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.fields.autospec_state.name == "Delivery status"'
  echo "$output" | jq -e '.fields.autospec_state.id | startswith("PVTSSF_")'
}

@test "a board with no candidate state field yields empty fields, not an error" {
  jq '{fields: [.fields[] | select(.name != "AutoSpec state" and .name != "Delivery status")]}' \
    "$FIX/p2-fields.json" > "$TMP/nofield.json"
  stub_gh "$TMP/nofield.json" "$FIX/p2-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/2 --emit plan
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.fields == {}'
}

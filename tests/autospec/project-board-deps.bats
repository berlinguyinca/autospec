#!/usr/bin/env bats
# Coverage for scripts/project-board-deps.sh — dependency extraction.

setup() {
  TMP="$(mktemp -d)"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-deps.sh"
  FIX="${BATS_TEST_DIRNAME}/../fixtures/project-board"
}
teardown() { rm -rf "$TMP"; }

item() { printf '{"items":[{"repo":"o/r","number":9,"body":%s,"dependencies":%s,"parent_issue":%s}]}' "$1" "${2:-null}" "${3:-null}"; }

@test "parses a bare '#N' blocked-by against the item's own repo" {
  item '"## Dependencies\n\n- Blocked by: #1 (IW-WB-000).\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == [{"repo":"o/r","number":1}]'
}

@test "parses a cross-repo 'owner/repo#N' blocked-by" {
  item '"## Dependencies\n\n- Blocked by: InferWeave/inferweave-protocol#42.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '.items[0].blocked_by == [{"repo":"InferWeave/inferweave-protocol","number":42}]'
}

@test "parses multiple blocked-by references on one line" {
  item '"## Dependencies\n\n- Blocked by: #1, #2, #3.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [1,2,3]'
}

@test "'Blocked by: none' yields an empty list" {
  item '"## Dependencies\n\n- Blocked by: none.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '.items[0].blocked_by == []'
}

@test "the Dependencies field wins over the body" {
  item '"## Dependencies\n\n- Blocked by: #99.\n"' '"#5"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [5]'
}

@test "the parent issue wins over the body when no Dependencies field is set" {
  item '"## Dependencies\n\n- Blocked by: #99.\n"' 'null' '"o/r#7"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [7]'
}

@test "a '#N' outside the Dependencies section is ignored" {
  item '"## Problem statement\n\nSee #123 for context.\n\n## Dependencies\n\n- Blocked by: none.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '.items[0].blocked_by == []'
}

@test "the real project-2 fixture yields blockers on 78 of 80 items" {
  jq '{items: [.items[] | {repo: .content.repository, number: .content.number, body: .content.body, dependencies: null, parent_issue: null}]}' \
     "$FIX/p2-items.json" > "$TMP/p2.json"
  run bash "$SCRIPT" < "$TMP/p2.json"
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.items[] | select((.blocked_by | length) > 0)] | length')" -eq 78 ]
}

# --- Degenerate input coverage ---------------------------------------------

@test "degenerate: stdin that is not JSON exits 0 with no crash" {
  printf 'not json at all {{{' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
}

@test "degenerate: JSON with no .items exits 0 and passes input through" {
  printf '{"foo":"bar"}' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.foo == "bar"'
}

@test "degenerate: an item with no body yields an empty blocked_by" {
  printf '{"items":[{"repo":"o/r","number":1}]}' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == []'
}

@test "degenerate: body: null yields an empty blocked_by" {
  printf '{"items":[{"repo":"o/r","number":1,"body":null}]}' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == []'
}

@test "degenerate: an item with no repo does not crash on a bare '#N'" {
  item='{"items":[{"number":1,"body":"## Dependencies\n\n- Blocked by: #4.\n"}]}'
  printf '%s' "$item" > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by[0].number == 4'
}

@test "degenerate: dependencies field is a non-string type (array) falls through" {
  item '"## Dependencies\n\n- Blocked by: #4.\n"' '[1,2,3]' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [4]'
}

@test "degenerate: parent_issue field is a non-string type (object) falls through" {
  item '"## Dependencies\n\n- Blocked by: #4.\n"' 'null' '{"foo":"bar"}' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [4]'
}

@test "degenerate: dependencies field is a non-string type (number) falls through" {
  item '"## Dependencies\n\n- Blocked by: #4.\n"' '123' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [4]'
}

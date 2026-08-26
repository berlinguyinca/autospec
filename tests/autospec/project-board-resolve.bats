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

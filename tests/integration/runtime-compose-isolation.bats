#!/usr/bin/env bats

setup() {
  export TEST_ROOT="$BATS_TEST_TMPDIR/runtime-compose-$BATS_TEST_NUMBER"
  export STATE_ROOT="$TEST_ROOT/state"
  export AUTOSPEC_BIN="$BATS_TEST_DIRNAME/../../target/debug/autospec"
  export FIXTURE="$BATS_TEST_DIRNAME/../fixtures/runtime-resources/compose-stack"
  mkdir -p "$TEST_ROOT/a" "$TEST_ROOT/b"
  cp -R "$FIXTURE/." "$TEST_ROOT/a"
  cp -R "$FIXTURE/." "$TEST_ROOT/b"
  export AGENT_ENV_STATE_ROOT="$STATE_ROOT"
}

teardown() {
  local failed=0
  if [ -n "${FORGED_VOLUME:-}" ]; then
    if ! docker volume rm "$FORGED_VOLUME" >/dev/null 2>&1; then
      failed=1
    fi
  fi
  if ! "$AUTOSPEC_BIN" runtime env down --repo "$TEST_ROOT/a" >/dev/null 2>&1; then
    failed=1
  fi
  if ! "$AUTOSPEC_BIN" runtime env down --repo "$TEST_ROOT/b" >/dev/null 2>&1; then
    failed=1
  fi
  return "$failed"
}

wait_for_http() {
  for _ in $(seq 1 50); do
    curl --fail --silent "$1" >/dev/null && return 0
    sleep 0.1
  done
  return 1
}

owned_count() {
  docker "$1" ls -q --filter "label=com.autospec.environment-id=$2" | wc -l
}

@test "Compose lifecycle isolates two real stacks, shares sessions, and leaves no owned resources" {
  [ -x "$AUTOSPEC_BIN" ]
  run docker compose version
  [ "$status" -eq 0 ]

  run "$AUTOSPEC_BIN" runtime env up --repo "$TEST_ROOT/a"
  [ "$status" -eq 0 ]
  dir_a="$(dirname "$(printf '%s\n' "$output" | sed -n 's/^AGENT_ENV_FILE=//p')")"
  run "$AUTOSPEC_BIN" runtime env up --repo "$TEST_ROOT/b"
  [ "$status" -eq 0 ]
  dir_b="$(dirname "$(printf '%s\n' "$output" | sed -n 's/^AGENT_ENV_FILE=//p')")"

  projects=($(find "$STATE_ROOT" -name inventory.json -exec jq -r .compose_project '{}' ';' | sort))
  [ "${#projects[@]}" -eq 2 ]
  [ "${projects[0]}" != "${projects[1]}" ]
  url_a="$(jq -r '.exports[0] | "http://\(.host):\(.port)"' "$dir_a/inventory.json")"
  url_b="$(jq -r '.exports[0] | "http://\(.host):\(.port)"' "$dir_b/inventory.json")"
  [ "$url_a" != "$url_b" ]
  wait_for_http "$url_a"
  wait_for_http "$url_b"
  [ "$(jq -r .networks[0] "$dir_a/inventory.json")" != "$(jq -r .networks[0] "$dir_b/inventory.json")" ]
  [ "$(jq -r .volumes[0].id "$dir_a/inventory.json")" != "$(jq -r .volumes[0].id "$dir_b/inventory.json")" ]
  [ "$(jq '.volumes | length' "$dir_a/inventory.json")" -eq 2 ]
  volumes_a=($(jq -r '.volumes[].id' "$dir_a/inventory.json"))
  volumes_b=($(jq -r '.volumes[].id' "$dir_b/inventory.json"))

  export FORGED_VOLUME="autospec-forged-anonymous-$BATS_TEST_NUMBER-$$"
  docker volume create "$FORGED_VOLUME" >/dev/null
  jq --arg id "$FORGED_VOLUME" '.volumes += [{logical_key:null,id:$id}]' \
    "$dir_a/inventory.json" > "$dir_a/inventory.forged.json"
  chmod 600 "$dir_a/inventory.forged.json"
  mv "$dir_a/inventory.forged.json" "$dir_a/inventory.json"
  run "$AUTOSPEC_BIN" runtime env gc --repo "$TEST_ROOT/a"
  [ "$status" -eq 2 ]
  [[ "$output" == RESOURCE_OWNER_MISMATCH:* ]]
  docker volume inspect "$FORGED_VOLUME" >/dev/null
  jq --arg id "$FORGED_VOLUME" '.volumes |= map(select(.id != $id))' \
    "$dir_a/inventory.json" > "$dir_a/inventory.restored.json"
  chmod 600 "$dir_a/inventory.restored.json"
  mv "$dir_a/inventory.restored.json" "$dir_a/inventory.json"
  docker volume rm "$FORGED_VOLUME" >/dev/null
  unset FORGED_VOLUME

  touch "$TEST_ROOT/release-never"
  "$AUTOSPEC_BIN" runtime env session --repo "$TEST_ROOT/a" -- sh -c \
    "touch '$TEST_ROOT/ready-1'; while [ ! -f '$TEST_ROOT/release-1' ]; do sleep 0.1; done" &
  first=$!
  "$AUTOSPEC_BIN" runtime env session --repo "$TEST_ROOT/a" -- sh -c \
    "touch '$TEST_ROOT/ready-2'; while [ ! -f '$TEST_ROOT/release-2' ]; do sleep 0.1; done" &
  second=$!
  for _ in $(seq 1 50); do
    [ -f "$TEST_ROOT/ready-1" ] && [ -f "$TEST_ROOT/ready-2" ] && break
    sleep 0.1
  done
  touch "$TEST_ROOT/release-1"
  wait "$first"
  wait_for_http "$url_a"
  touch "$TEST_ROOT/release-2"
  wait "$second"

  run "$AUTOSPEC_BIN" runtime env down --repo "$TEST_ROOT/b"
  [ "$status" -eq 0 ]
  for directory in "$dir_a" "$dir_b"; do
    environment_id="$(basename "$directory")"
    [ "$(docker ps -aq --filter "label=com.autospec.environment-id=$environment_id" | wc -l)" -eq 0 ]
    [ "$(owned_count network "$environment_id")" -eq 0 ]
    [ "$(owned_count volume "$environment_id")" -eq 0 ]
  done
  for volume in "${volumes_a[@]}" "${volumes_b[@]}"; do
    run docker volume inspect "$volume"
    [ "$status" -ne 0 ]
  done
}

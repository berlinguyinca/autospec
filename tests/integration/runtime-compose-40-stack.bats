#!/usr/bin/env bats

setup() {
  export REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  export AUTOSPEC_BIN="$REPO_ROOT/target/debug/autospec"
  export FIXTURE="$REPO_ROOT/tests/fixtures/runtime-resources/forty-stack"
  export TEST_ROOT="$BATS_TEST_TMPDIR/runtime-compose-40-$BATS_TEST_NUMBER"
  export STATE_ROOT="$TEST_ROOT/state"
  export AGENT_ENV_STATE_ROOT="$STATE_ROOT"
  export FORTY_STACK_IMAGE="autospec/runtime-isolation-forty:$BATS_TEST_NUMBER-$$"
  export HOST_DOCKER_ENDPOINT="${DOCKER_HOST:-}"
  export ISOLATED_ENGINE="autospec-forty-engine-$BATS_TEST_NUMBER-$$"
  export REPOS_FILE="$TEST_ROOT/repos"
  export PROJECTS_FILE="$TEST_ROOT/projects"
  export ENVIRONMENTS_FILE="$TEST_ROOT/environments"
  export ROWS_FILE="$TEST_ROOT/rows.ndjson"
  export CSV_FILE="$TEST_ROOT/compose-40-stack.csv"
  export JSON_FILE="$TEST_ROOT/compose-40-stack.json"
  mkdir -p "$TEST_ROOT/logs"
  : > "$REPOS_FILE"
  : > "$PROJECTS_FILE"
  : > "$ENVIRONMENTS_FILE"
  : > "$ROWS_FILE"
}

teardown() {
  local failed=0
  if ! cleanup_stacks; then
    failed=1
    sed -n '1,200p' "$TEST_ROOT/cleanup.log" >&2
  fi
  if ! remove_fixture_image; then
    failed=1
  fi
  if ! remove_isolated_engine; then
    failed=1
  fi
  return "$failed"
}

now_ms() {
  python3 -c 'import time; print(time.time_ns() // 1000000)'
}

require_engine_and_fixture() {
  [ -x "$AUTOSPEC_BIN" ]
  host_docker compose version
  host_docker info >/dev/null
  [ -f "$FIXTURE/Dockerfile" ]
  [ -f "$FIXTURE/compose.yaml" ]
  [ -f "$FIXTURE/.autospec/runtime.yml" ]
}

host_docker() {
  if [ -n "$HOST_DOCKER_ENDPOINT" ]; then
    DOCKER_HOST="$HOST_DOCKER_ENDPOINT" docker "$@"
  else
    env -u DOCKER_HOST docker "$@"
  fi
}

start_isolated_engine() {
  mkdir -p "$TEST_ROOT/docker-run"
  host_docker run -d --privileged --name "$ISOLATED_ENGINE" \
    -v "$TEST_ROOT/docker-run:/var/run" docker:29.1.3-dind \
    --host=unix:///var/run/docker.sock \
    --default-address-pool=base=10.240.0.0/16,size=24 > "$TEST_ROOT/engine-id"
  export DOCKER_HOST="unix://$TEST_ROOT/docker-run/docker.sock"
  for _ in $(seq 1 300); do
    if host_docker exec "$ISOLATED_ENGINE" test -S /var/run/docker.sock; then
      host_docker exec "$ISOLATED_ENGINE" chgrp "$(id -g)" /var/run/docker.sock
      host_docker exec "$ISOLATED_ENGINE" chmod 660 /var/run/docker.sock
    fi
    docker info >/dev/null 2>&1 && return 0
    sleep 0.1
  done
  host_docker logs "$ISOLATED_ENGINE" >&2
  return 1
}

remove_isolated_engine() {
  if host_docker container inspect "${ISOLATED_ENGINE:-missing}" >/dev/null 2>&1; then
    host_docker rm -f "$ISOLATED_ENGINE" >> "$TEST_ROOT/cleanup.log" 2>&1
    host_docker run --rm --name "${ISOLATED_ENGINE}-cleanup" \
      -v "$TEST_ROOT/docker-run:/cleanup" --entrypoint sh docker:29.1.3-dind \
      -c 'rm -rf /cleanup/* /cleanup/.[!.]* /cleanup/..?*' \
      >> "$TEST_ROOT/cleanup.log" 2>&1
  fi
}

build_fixture_image() {
  docker build -q -t "$FORTY_STACK_IMAGE" "$FIXTURE" > "$TEST_ROOT/image-id"
}

create_linked_worktrees() {
  local seed="$TEST_ROOT/seed"
  mkdir -p "$seed"
  cp -R "$FIXTURE/." "$seed"
  git -C "$seed" init -q -b main
  git -C "$seed" config user.email autospec-test@example.invalid
  git -C "$seed" config user.name "Autospec Integration"
  git -C "$seed" add .
  git -C "$seed" commit -qm "test fixture"
  for index in $(seq -w 1 40); do
    local worktree="$TEST_ROOT/worktree-$index"
    git -C "$seed" worktree add -q --detach "$worktree" HEAD
    printf '%s\n' "$worktree" >> "$REPOS_FILE"
  done
}

start_stacks_concurrently() {
  : > "$TEST_ROOT/pids"
  local index=0
  while IFS= read -r repo; do
    index=$((index + 1))
    "$AUTOSPEC_BIN" runtime env up --repo "$repo" \
      > "$TEST_ROOT/logs/$index.out" 2> "$TEST_ROOT/logs/$index.err" &
    printf '%s %s\n' "$!" "$index" >> "$TEST_ROOT/pids"
  done < "$REPOS_FILE"
  local failures=0
  while read -r pid index; do
    if ! wait "$pid"; then
      failures=$((failures + 1))
      sed -n '1,120p' "$TEST_ROOT/logs/$index.err" >&2
    fi
  done < "$TEST_ROOT/pids"
  [ "$failures" -eq 0 ]
}

wait_for_http_status() {
  local url="$1"
  local status=000
  for _ in $(seq 1 100); do
    if host_docker exec "$ISOLATED_ENGINE" wget -q -O /dev/null "$url"; then
      status=200
    else
      status=000
    fi
    [ "$status" = 200 ] && break
    sleep 0.1
  done
  printf '%s\n' "$status"
}

collect_rows() {
  printf '%s\n' 'environment_id,compose_project,container_id,network_id,volume_id,host_port,http_status' > "$CSV_FILE"
  local index=0
  while IFS= read -r repo; do
    index=$((index + 1))
    local env_file inventory environment project container network volume port url status
    env_file="$(sed -n 's/^AGENT_ENV_FILE=//p' "$TEST_ROOT/logs/$index.out")"
    inventory="$(dirname "$env_file")/inventory.json"
    environment="$(basename "$(dirname "$env_file")")"
    project="$(jq -er .compose_project "$inventory")"
    container="$(jq -er .containers[0] "$inventory")"
    network="$(jq -er .networks[0] "$inventory")"
    volume="$(jq -er .volumes[0].id "$inventory")"
    port="$(jq -er .exports[0].port "$inventory")"
    url="$(jq -er '.exports[0] | "http://\(.host):\(.port)"' "$inventory")"
    status="$(wait_for_http_status "$url")"
    printf '%s,%s,%s,%s,%s,%s,%s\n' "$environment" "$project" "$container" "$network" "$volume" "$port" "$status" >> "$CSV_FILE"
    jq -cn --arg environment_id "$environment" --arg compose_project "$project" \
      --arg container_id "$container" --arg network_id "$network" --arg volume_id "$volume" \
      --argjson host_port "$port" --argjson http_status "$status" \
      '{environment_id:$environment_id,compose_project:$compose_project,container_id:$container_id,network_id:$network_id,volume_id:$volume_id,host_port:$host_port,http_status:$http_status}' >> "$ROWS_FILE"
    printf '%s\n' "$environment" >> "$ENVIRONMENTS_FILE"
    printf '%s %s\n' "$repo" "$project" >> "$PROJECTS_FILE"
  done < "$REPOS_FILE"
}

assert_unique_rows() {
  [ "$(tail -n +2 "$CSV_FILE" | wc -l)" -eq 40 ]
  for column in 1 2 3 4 5 6; do
    [ "$(tail -n +2 "$CSV_FILE" | cut -d, -f"$column" | sort -u | wc -l)" -eq 40 ]
  done
  [ "$(tail -n +2 "$CSV_FILE" | cut -d, -f7 | sort -u)" = 200 ]
}

exercise_reference_survival() {
  local repo url first second
  repo="$(sed -n '1p' "$REPOS_FILE")"
  url="http://127.0.0.1:$(sed -n '2p' "$CSV_FILE" | cut -d, -f6)"
  "$AUTOSPEC_BIN" runtime env session --repo "$repo" -- sh -c \
    "touch '$TEST_ROOT/session-1'; while [ ! -f '$TEST_ROOT/release-1' ]; do sleep 0.05; done" &
  first=$!
  "$AUTOSPEC_BIN" runtime env session --repo "$repo" -- sh -c \
    "touch '$TEST_ROOT/session-2'; while [ ! -f '$TEST_ROOT/release-2' ]; do sleep 0.05; done" &
  second=$!
  for _ in $(seq 1 100); do
    [ -f "$TEST_ROOT/session-1" ] && [ -f "$TEST_ROOT/session-2" ] && break
    sleep 0.05
  done
  touch "$TEST_ROOT/release-1"
  wait "$first"
  [ "$(wait_for_http_status "$url")" = 200 ]
  touch "$TEST_ROOT/release-2"
  wait "$second"
}

exercise_provisioning_recovery() {
  local repo project environment inventory owner
  read -r repo project < <(sed -n '2p' "$PROJECTS_FILE")
  environment="$(sed -n '2p' "$ENVIRONMENTS_FILE")"
  inventory="$STATE_ROOT/$environment/inventory.json"
  owner="$STATE_ROOT/$environment/owner.json"
  docker compose -p "$project" -f "$repo/compose.yaml" down -v --remove-orphans
  jq '.compose_project=null | .containers=[] | .networks=[] | .volumes=[] | .exports=[]' "$inventory" > "$inventory.next"
  mv "$inventory.next" "$inventory"
  jq '.lifecycle="Provisioning"' "$owner" > "$owner.next"
  mv "$owner.next" "$owner"
  chmod 600 "$inventory" "$owner"
  rm "$STATE_ROOT/$environment/env"
  "$AUTOSPEC_BIN" runtime env up --repo "$repo" > "$TEST_ROOT/recovery.out"
  local recovered_url
  recovered_url="$(jq -r '.exports[0] | "http://\(.host):\(.port)"' "$inventory")"
  [ "$(wait_for_http_status "$recovered_url")" = 200 ]
}

exercise_teardown_recovery() {
  local repo environment owner
  repo="$(sed -n '3p' "$REPOS_FILE")"
  environment="$(sed -n '3p' "$ENVIRONMENTS_FILE")"
  owner="$STATE_ROOT/$environment/owner.json"
  jq '.lifecycle="TearingDown"' "$owner" > "$owner.next"
  mv "$owner.next" "$owner"
  "$AUTOSPEC_BIN" runtime env down --repo "$repo"
  [ ! -e "$owner" ]
}

down_non_recovery_stacks() {
  local index=0 failures=0 repo
  while IFS= read -r repo; do
    index=$((index + 1))
    [ "$index" -le 3 ] && continue
    if ! "$AUTOSPEC_BIN" runtime env down --repo "$repo" >> "$TEST_ROOT/cleanup.log" 2>&1; then
      failures=$((failures + 1))
    fi
  done < "$REPOS_FILE"
  [ "$failures" -eq 0 ]
}

cleanup_stacks() {
  local failures=0
  if [ -s "${REPOS_FILE:-/nonexistent}" ]; then
    while IFS= read -r repo; do
      if ! "$AUTOSPEC_BIN" runtime env down --repo "$repo" >> "$TEST_ROOT/cleanup.log" 2>&1; then
        failures=$((failures + 1))
      fi
    done < "$REPOS_FILE"
  fi
  if [ -s "${PROJECTS_FILE:-/nonexistent}" ]; then
    while read -r repo project; do
      if ! docker compose -p "$project" -f "$repo/compose.yaml" down -v --remove-orphans >> "$TEST_ROOT/cleanup.log" 2>&1; then
        failures=$((failures + 1))
      fi
    done < "$PROJECTS_FILE"
  fi
  printf '%s\n' "$failures" > "$TEST_ROOT/cleanup-errors"
  [ "$failures" -eq 0 ]
}

remove_fixture_image() {
  if docker image inspect "${FORTY_STACK_IMAGE:-missing}" >/dev/null 2>&1; then
    docker image rm "$FORTY_STACK_IMAGE" >> "$TEST_ROOT/cleanup.log" 2>&1
  fi
}

count_running_containers() {
  local count=0 environment
  while IFS= read -r environment; do
    count=$((count + $(docker ps -q --filter "label=com.autospec.environment-id=$environment" | wc -l)))
  done < "$ENVIRONMENTS_FILE"
  printf '%s\n' "$count"
}

count_leaks() {
  local leaks=0 environment
  while IFS= read -r environment; do
    leaks=$((leaks + $(docker ps -aq --filter "label=com.autospec.environment-id=$environment" | wc -l)))
    leaks=$((leaks + $(docker network ls -q --filter "label=com.autospec.environment-id=$environment" | wc -l)))
    leaks=$((leaks + $(docker volume ls -q --filter "label=com.autospec.environment-id=$environment" | wc -l)))
  done < "$ENVIRONMENTS_FILE"
  printf '%s\n' "$leaks"
}

write_report() {
  local startup_ms="$1" teardown_ms="$2" peak="$3" collisions="$4" retries="$5" leaks="$6" cleanup_errors="$7"
  jq -s --argjson startup_duration_ms "$startup_ms" --argjson teardown_duration_ms "$teardown_ms" \
    --argjson peak_containers "$peak" --argjson collisions "$collisions" \
    --argjson retry_exhaustions "$retries" --argjson leaked_resources "$leaks" --argjson cleanup_errors "$cleanup_errors" \
    '{stack_count:length,peak_containers:$peak_containers,startup_duration_ms:$startup_duration_ms,teardown_duration_ms:$teardown_duration_ms,collisions:$collisions,retry_exhaustions:$retry_exhaustions,leaked_resources:$leaked_resources,cleanup_errors:$cleanup_errors,reference_count_survival:true,provisioning_recovery:true,teardown_recovery:true,rows:.}' \
    "$ROWS_FILE" > "$JSON_FILE"
  local report_dir="$REPO_ROOT/reports/runtime-isolation"
  mkdir -p "$report_dir"
  if [ ! -e "$report_dir/compose-40-stack.csv" ] || [ "${AUTOSPEC_RUNTIME_REPORT_REFRESH:-0}" = 1 ]; then
    cp "$CSV_FILE" "$report_dir/compose-40-stack.csv"
    cp "$JSON_FILE" "$report_dir/compose-40-stack.json"
  fi
}

@test "forty linked worktrees run isolated real Compose stacks and clean every owned resource" {
  require_engine_and_fixture
  start_isolated_engine
  build_fixture_image
  create_linked_worktrees
  local started startup_ms peak teardown_started teardown_ms collisions retries leaks cleanup_errors
  started="$(now_ms)"
  start_stacks_concurrently
  collect_rows
  startup_ms=$(( $(now_ms) - started ))
  assert_unique_rows
  peak="$(count_running_containers)"
  teardown_started="$(now_ms)"
  exercise_reference_survival
  down_non_recovery_stacks
  exercise_provisioning_recovery
  exercise_teardown_recovery
  cleanup_stacks
  teardown_ms=$(( $(now_ms) - teardown_started ))
  collisions="$(grep -Rho 'PORT_ALREADY_CLAIMED' "$TEST_ROOT/logs" | wc -l)"
  retries="$(grep -Rho 'RETRIES_EXHAUSTED' "$TEST_ROOT/logs" | wc -l)"
  leaks="$(count_leaks)"
  cleanup_errors="$(cat "$TEST_ROOT/cleanup-errors")"
  [ "$peak" -eq 40 ]
  [ "$collisions" -eq 0 ]
  [ "$retries" -eq 0 ]
  [ "$leaks" -eq 0 ]
  [ "$cleanup_errors" -eq 0 ]
  write_report "$startup_ms" "$teardown_ms" "$peak" "$collisions" "$retries" "$leaks" "$cleanup_errors"
  jq -e '.stack_count == 40 and .peak_containers == 40 and .cleanup_errors == 0' "$JSON_FILE"
  remove_fixture_image
}

@test "public runtime documentation names commands, v2 ownership, opt-outs, and recovery" {
  local docs="$REPO_ROOT/README.md $REPO_ROOT/AGENTS.md $REPO_ROOT/docs/cli-reference.md $REPO_ROOT/docs/CONFIG_REFERENCE.md $REPO_ROOT/docs/USER_MANUAL.md $REPO_ROOT/docs/runbooks/agent-runtime-manifest.md $REPO_ROOT/docs/runbooks/agent-runtime-companion-stacks.md"
  for term in 'normalize-compose' 'down --purge-maven' 'AUTOSPEC_MAVEN_ISOLATION=off' 'AUTOSPEC_COMPOSE_ISOLATION=off' 'AUTOSPEC_ENV_DISABLE=1' 'AUTOSPEC_ISOLATION_BYPASSED=1' 'RUNTIME_STATE_SYMLINK_REJECTED'; do
    grep -F "$term" $docs >/dev/null
  done
  grep -F 'version: 2' $docs >/dev/null
  grep -F 'verified' $docs >/dev/null
}

#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
SCRIPT="$REPO_ROOT/scripts/autospec-control-plane.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  PROJECT_DIR="$TEST_TMP/project"
  GH_REMOTE_ROOT="$TEST_TMP/remotes"
  GH_LOG="$TEST_TMP/gh.log"
  mkdir -p "$TEST_TMP/bin" "$PROJECT_DIR/.autospec" "$GH_REMOTE_ROOT"
  cat > "$TEST_TMP/bin/gh" <<'GH'
#!/usr/bin/env bash
set -eu
printf 'gh %s\n' "$*" >> "$GH_LOG"
if [ "${1:-}" = "repo" ] && [ "${2:-}" = "view" ]; then
  full="${3:-}"
  if [ -d "$GH_REMOTE_ROOT/${full}.git" ]; then
    printf 'file://%s/%s.git\n' "$GH_REMOTE_ROOT" "$full"
    exit 0
  fi
  exit 1
fi
if [ "${1:-}" = "repo" ] && [ "${2:-}" = "create" ]; then
  full="${3:-}"
  mkdir -p "$GH_REMOTE_ROOT/$(dirname "$full")"
  git init --bare "$GH_REMOTE_ROOT/${full}.git" >/dev/null
  printf 'file://%s/%s.git\n' "$GH_REMOTE_ROOT" "$full"
  exit 0
fi
printf 'unexpected gh invocation: %s\n' "$*" >&2
exit 99
GH
  chmod +x "$TEST_TMP/bin/gh"
  cat > "$TEST_TMP/bin/autospec" <<'AUTOSPEC'
#!/usr/bin/env bash
set -eu
[ "${1:-}" = "project" ] && [ "${2:-}" = "onboard" ] || exit 99
printf '%s\n' '{"outcome":"reconciled","pending_projection":0}'
AUTOSPEC
  chmod +x "$TEST_TMP/bin/autospec"
  export GH_LOG GH_REMOTE_ROOT
  export PATH="$TEST_TMP/bin:$PATH"
  export GIT_AUTHOR_NAME="Autospec Test"
  export GIT_AUTHOR_EMAIL="autospec-test@example.invalid"
  export GIT_COMMITTER_NAME="Autospec Test"
  export GIT_COMMITTER_EMAIL="autospec-test@example.invalid"
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "bootstrap --confirm requires explicit owner and repo names" {
  cd "$PROJECT_DIR"
  run bash "$SCRIPT" bootstrap --confirm --owner test-owner --governance-repo gov
  [ "$status" -eq 2 ]
  [[ "$output" == *"--confirm requires --owner, --governance-repo, and --observatory-repo"* ]]
  [ ! -s "$GH_LOG" ]
}

@test "bootstrap --confirm rejects unsafe repo path names before gh" {
  cd "$PROJECT_DIR"
  run bash "$SCRIPT" bootstrap --confirm \
    --owner test-owner \
    --governance-repo ../unsafe \
    --observatory-repo autospec-observatory

  [ "$status" -eq 2 ]
  [[ "$output" == *"repo names must"* ]]
  [ ! -s "$GH_LOG" ]
}

@test "bootstrap --confirm creates companion repos and writes control-plane config" {
  cd "$PROJECT_DIR"
  run bash "$SCRIPT" bootstrap --confirm \
    --owner test-owner \
    --governance-repo autospec-governance \
    --observatory-repo autospec-observatory

  [ "$status" -eq 0 ]
  [[ "$output" == *"Control plane bootstrap completed"* ]]
  [ -d "$GH_REMOTE_ROOT/test-owner/autospec-governance.git" ]
  [ -d "$GH_REMOTE_ROOT/test-owner/autospec-observatory.git" ]
  [ -s ".autospec/control-plane.json" ]
  jq -e '.owner == "test-owner"' .autospec/control-plane.json >/dev/null
  jq -e '.governance.url | startswith("file://")' .autospec/control-plane.json >/dev/null
  jq -e '.observatory.url | startswith("file://")' .autospec/control-plane.json >/dev/null
  jq -e '.bootstrap.confirmed == true and (.bootstrap.completed_at | length > 0)' .autospec/control-plane.json >/dev/null
  git --git-dir "$GH_REMOTE_ROOT/test-owner/autospec-governance.git" log --oneline --all | grep -Fq 'feat: scaffold autospec governance repo'
  git --git-dir "$GH_REMOTE_ROOT/test-owner/autospec-observatory.git" log --oneline --all | grep -Fq 'feat: scaffold autospec observatory repo'
}

@test "bootstrap --confirm emits completed event when observatory endpoint is configured" {
  cd "$PROJECT_DIR"
  export AUTOSPEC_OBSERVATORY_URL="http://127.0.0.1:9"
  export AUTOSPEC_OBSERVATORY_OFFLINE=1
  export AUTOSPEC_OBSERVATORY_DIR="$TEST_TMP/observatory"
  export AUTOSPEC_RUN_ID="control-plane-bootstrap-test"
  export AUTOSPEC_WORKER_ID="worker-control-plane-test"

  run bash "$SCRIPT" bootstrap --confirm \
    --owner test-owner \
    --governance-repo autospec-governance \
    --observatory-repo autospec-observatory

  [ "$status" -eq 0 ]
  outbox="$AUTOSPEC_OBSERVATORY_DIR/outbox/$AUTOSPEC_RUN_ID.jsonl"
  [ -s "$outbox" ]
  jq -e 'select(.event_type == "ControlPlaneBootstrapCompleted" and .repository_id == "test-owner/autospec")' "$outbox" >/dev/null
}

#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
SCRIPT="$REPO_ROOT/scripts/autospec-policy-resolver.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo/.autospec" "$TEST_TMP/governance/policies"
}

teardown() {
  rm -rf "$TEST_TMP"
}

write_policy() {
  local path="$1"
  cat > "$path" <<'YAML'
policy_id: governance-private-company-default
policy_version: 2026.07.08
project_class: private-company
privacy_tier: summary
raw_logs_allowed: false
YAML
}

@test "repo-local autonomous policy emits deterministic resolution trace" {
  cat > "$TEST_TMP/repo/.autospec/autonomous.yml" <<'YAML'
policy_id: repo-local-test
policy_version: 1.2.3
policy_digest: sha256:repo-digest
project_class: client-project
privacy_tier: metadata-only
raw_logs_allowed: false
YAML

  run bash "$SCRIPT" --repo "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q '"policy_source":"repo-local"'
  printf '%s\n' "$output" | grep -q '"policy_id":"repo-local-test"'
  printf '%s\n' "$output" | grep -q '"policy_resolution_trace"'
  printf '%s\n' "$output" | grep -q 'repo-local:.autospec/autonomous.yml:hit'
}

@test "governance default validates policy digest before trust" {
  policy="$TEST_TMP/governance/policies/private-company-default.yml"
  write_policy "$policy"
  digest="sha256:$(shasum -a 256 "$policy" | awk '{print $1}')"

  run bash "$SCRIPT" --repo "$TEST_TMP/repo" --governance-dir "$TEST_TMP/governance" --project-class private-company --expected-policy-digest "$digest"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q '"policy_source":"governance-default"'
  printf '%s\n' "$output" | grep -q '"policy_digest":"'"$digest"'"'

  run bash "$SCRIPT" --repo "$TEST_TMP/repo" --governance-dir "$TEST_TMP/governance" --project-class private-company --expected-policy-digest sha256:bad
  [ "$status" -ne 0 ]
  printf '%s\n' "$output" | grep -q 'policy digest mismatch'
}

@test "observatory assignment is used before governance defaults" {
  cat > "$TEST_TMP/assignment.yml" <<'YAML'
policy_id: observatory-assigned
policy_version: 2026.07.09
project_class: research
privacy_tier: summary
raw_logs_allowed: false
YAML
  policy="$TEST_TMP/governance/policies/research-default.yml"
  cat > "$policy" <<'YAML'
policy_id: governance-research-default
policy_version: 2026.07.08
project_class: research
privacy_tier: evidence
raw_logs_allowed: false
YAML

  run bash "$SCRIPT" --repo "$TEST_TMP/repo" --observatory-assignment "$TEST_TMP/assignment.yml" --governance-dir "$TEST_TMP/governance" --project-class research
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q '"policy_source":"observatory-assignment"'
  printf '%s\n' "$output" | grep -q '"policy_id":"observatory-assigned"'
  printf '%s\n' "$output" | grep -q 'observatory-assignment:assignment.yml:hit'
}

@test "metadata-only event scrubber excludes artifact details" {
  cat > "$TEST_TMP/event.json" <<'JSON'
{"event_type":"WorkItemFinished","repo":"private/repo","summary":"done","artifact_details":{"path":"reports/raw.log","sha":"abc"},"raw_logs":"secret log"}
JSON

  run bash "$SCRIPT" --repo "$TEST_TMP/repo" --project-class client-project --event-file "$TEST_TMP/event.json"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q '"privacy_tier":"metadata-only"'
  ! printf '%s\n' "$output" | grep -q 'artifact_details'
  ! printf '%s\n' "$output" | grep -q 'secret log'
}

@test "full-debug raw logs are rejected unless fixture key allows them" {
  cat > "$TEST_TMP/repo/.autospec/autonomous.yml" <<'YAML'
policy_id: sandbox-debug
policy_version: 1
project_class: sandbox
privacy_tier: full-debug
raw_logs_allowed: true
YAML
  cat > "$TEST_TMP/event.json" <<'JSON'
{"event_type":"RunLog","summary":"debug","raw_logs":"full raw log"}
JSON

  run bash "$SCRIPT" --repo "$TEST_TMP/repo" --event-file "$TEST_TMP/event.json" --api-key-privacy-tier evidence
  [ "$status" -ne 0 ]
  printf '%s\n' "$output" | grep -q 'event exceeds api key privacy tier'

  run bash "$SCRIPT" --repo "$TEST_TMP/repo" --event-file "$TEST_TMP/event.json" --api-key-privacy-tier full-debug --allow-full-debug-raw-logs
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q '"raw_logs":"full raw log"'
}

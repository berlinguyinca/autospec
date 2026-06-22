#!/usr/bin/env bats
#
# test_qa_deploy_schema.bats — schema-shape coverage for the qa-deploy contract
# (autospec issue #1292, part of #694).
#
# Validates schemas/autospec-qa-deploy.schema.json against the fixtures under
# tests/qa/fixtures/ using the SAME toolchain as
# skills/autospec-test/scripts/load-contract.sh: yq (YAML->JSON) -> ajv.
#
# Scope of THIS test (schema only):
#   - schema is valid JSON and compiles with `ajv compile`
#   - the valid fixture validates (exit 0)
#   - every STRUCTURALLY-invalid fixture is rejected by the schema (exit != 0)
#   - malformed YAML fails at the yq parse step (before schema validation)
#
# Out of scope (covered by the runner bats in #1293): the safety-floor rules
# (forbidden-target match, production-pattern rejection, max_records required
# for data-clone stages). Those are runner regex checks, NOT JSON Schema, so
# tests/qa/fixtures/invalid-clone-missing-max-records.yml is intentionally
# schema-VALID here.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SCHEMA="$REPO_ROOT/schemas/autospec-qa-deploy.schema.json"
  FIXTURES="$REPO_ROOT/tests/qa/fixtures"
  TMPDIR_TEST="$(mktemp -d /tmp/autospec-qa-deploy-schema-XXXXXX)"
}

teardown() {
  rm -rf "$TMPDIR_TEST"
}

# Convert a YAML fixture to JSON on disk, returning the JSON path.
# Skips the test if yq is unavailable.
_yaml_to_json() {
  local yml="$1" out="$2"
  command -v yq >/dev/null 2>&1 || skip "yq not available (brew install yq)"
  yq -o=json '.' "$yml" > "$out"
}

@test "qa-deploy schema file exists and is valid JSON" {
  [ -f "$SCHEMA" ]
  command -v python3 >/dev/null 2>&1 || skip "python3 not available"
  run python3 -m json.tool "$SCHEMA"
  [ "$status" -eq 0 ]
}

@test "qa-deploy schema compiles with ajv" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (npm install -g ajv-cli)"
  run ajv compile -s "$SCHEMA" --spec=draft2020
  [ "$status" -eq 0 ]
}

@test "valid fixture passes schema validation" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (npm install -g ajv-cli)"
  _yaml_to_json "$FIXTURES/valid.yml" "$TMPDIR_TEST/valid.json"
  run ajv validate -s "$SCHEMA" --spec=draft2020 -d "$TMPDIR_TEST/valid.json"
  [ "$status" -eq 0 ]
}

@test "canonical .autospec/qa-deploy.yml fixture passes schema validation" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (npm install -g ajv-cli)"
  _yaml_to_json "$FIXTURES/.autospec/qa-deploy.yml" "$TMPDIR_TEST/canonical.json"
  run ajv validate -s "$SCHEMA" --spec=draft2020 -d "$TMPDIR_TEST/canonical.json"
  [ "$status" -eq 0 ]
}

@test "fixture missing top-level stages is rejected" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (npm install -g ajv-cli)"
  _yaml_to_json "$FIXTURES/invalid-missing-stages.yml" "$TMPDIR_TEST/x.json"
  run ajv validate -s "$SCHEMA" --spec=draft2020 -d "$TMPDIR_TEST/x.json"
  [ "$status" -ne 0 ]
}

@test "fixture missing target_envs.forbidden is rejected" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (npm install -g ajv-cli)"
  _yaml_to_json "$FIXTURES/invalid-missing-forbidden.yml" "$TMPDIR_TEST/x.json"
  run ajv validate -s "$SCHEMA" --spec=draft2020 -d "$TMPDIR_TEST/x.json"
  [ "$status" -ne 0 ]
}

@test "fixture with empty target_envs.forbidden is rejected (>=1 entry required)" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (npm install -g ajv-cli)"
  _yaml_to_json "$FIXTURES/invalid-empty-forbidden.yml" "$TMPDIR_TEST/x.json"
  run ajv validate -s "$SCHEMA" --spec=draft2020 -d "$TMPDIR_TEST/x.json"
  [ "$status" -ne 0 ]
}

@test "fixture with a stage missing command is rejected" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (npm install -g ajv-cli)"
  _yaml_to_json "$FIXTURES/invalid-stage-missing-command.yml" "$TMPDIR_TEST/x.json"
  run ajv validate -s "$SCHEMA" --spec=draft2020 -d "$TMPDIR_TEST/x.json"
  [ "$status" -ne 0 ]
}

@test "fixture with a health_check missing url is rejected" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (npm install -g ajv-cli)"
  _yaml_to_json "$FIXTURES/invalid-healthcheck-missing-url.yml" "$TMPDIR_TEST/x.json"
  run ajv validate -s "$SCHEMA" --spec=draft2020 -d "$TMPDIR_TEST/x.json"
  [ "$status" -ne 0 ]
}

@test "malformed YAML fixture fails to parse at the yq step" {
  command -v yq >/dev/null 2>&1 || skip "yq not available (brew install yq)"
  run yq -o=json '.' "$FIXTURES/invalid-malformed-yaml.yml"
  [ "$status" -ne 0 ]
}

@test "data-clone-without-max_records fixture is schema-valid (runner enforces the floor, #1293)" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (npm install -g ajv-cli)"
  _yaml_to_json "$FIXTURES/invalid-clone-missing-max-records.yml" "$TMPDIR_TEST/x.json"
  run ajv validate -s "$SCHEMA" --spec=draft2020 -d "$TMPDIR_TEST/x.json"
  [ "$status" -eq 0 ]
}

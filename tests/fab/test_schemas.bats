#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  MODEL_SCHEMA="$REPO_ROOT/schemas/autospec-fab-model.schema.json"
  GATE_SCHEMA="$REPO_ROOT/schemas/autospec-fab-release-gate.schema.json"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-fab-schemas-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

@test "fab-model schema accepts a minimal valid model sidecar" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TEST_TMPDIR/model-ok.json" <<'JSON'
{
  "material": "PETG",
  "print_orientation": "flat",
  "load_critical": false,
  "flow_critical": true,
  "dims": { "x_mm": 80, "y_mm": 60, "z_mm": 40 },
  "ports": [
    { "id": "inlet", "type": "npt", "size": "1/4", "clearance_mm": 6.0 }
  ],
  "gaskets": [
    { "id": "main_seal", "groove": "o-ring", "surrounding_wall_mm": 5.5 }
  ]
}
JSON

  run ajv validate -s "$MODEL_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/model-ok.json"
  [ "$status" -eq 0 ]
}

@test "fab-model schema accepts a sidecar carrying engine-read name + body_count" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  # The engine (stl-release-gate.py _model_name) reads "name" and
  # stage-geometry.py reads "body_count" off the model sidecar. A real sidecar
  # carrying either must NOT be rejected by the additionalProperties:false root.
  cat > "$TEST_TMPDIR/model-named.json" <<'JSON'
{
  "name": "DogfoodFeaturefulManifold",
  "body_count": 1,
  "material": "PETG",
  "print_orientation": "flat",
  "load_critical": false,
  "flow_critical": true
}
JSON

  run ajv validate -s "$MODEL_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/model-named.json"
  [ "$status" -eq 0 ]
}

@test "fab-model schema rejects a sidecar missing required field material" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TEST_TMPDIR/model-bad.json" <<'JSON'
{
  "print_orientation": "flat",
  "load_critical": false,
  "flow_critical": true
}
JSON

  run ajv validate -s "$MODEL_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/model-bad.json"
  [ "$status" -ne 0 ]
}

@test "fab-release-gate schema accepts a minimal valid release-gate" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TEST_TMPDIR/gate-ok.json" <<'JSON'
{
  "schema_version": 1,
  "stages": [
    { "stage": "geometry", "status": "pass", "detail": "Watertight and single body" }
  ]
}
JSON

  run ajv validate -s "$GATE_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/gate-ok.json"
  [ "$status" -eq 0 ]
}

@test "fab-release-gate schema rejects a stage with an invalid status enum value" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TEST_TMPDIR/gate-bad-status.json" <<'JSON'
{
  "schema_version": 1,
  "stages": [
    { "stage": "geometry", "status": "error" }
  ]
}
JSON

  run ajv validate -s "$GATE_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/gate-bad-status.json"
  [ "$status" -ne 0 ]
}

@test "fab-release-gate schema rejects a gate with empty stages array" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TEST_TMPDIR/gate-empty-stages.json" <<'JSON'
{
  "schema_version": 1,
  "stages": []
}
JSON

  run ajv validate -s "$GATE_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/gate-empty-stages.json"
  [ "$status" -ne 0 ]
}

@test "fab-release-gate schema rejects a gate missing stages entirely" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TEST_TMPDIR/gate-no-stages.json" <<'JSON'
{
  "schema_version": 1
}
JSON

  run ajv validate -s "$GATE_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/gate-no-stages.json"
  [ "$status" -ne 0 ]
}

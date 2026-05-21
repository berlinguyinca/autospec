#!/usr/bin/env bats
# contract-loader-v2.bats — TDD tests for v2 contract extension (phase 1)
#
# Tests validate that:
#   1. v2 fixture files are accepted/rejected by validate-contract.sh with correct exit codes
#   2. ajv schema validation matches expected accept/reject for each fixture
#   3. v1 fixtures still pass (backward compatibility)
#
# Exit codes from validate-contract.sh:
#   0 = valid
#   1 = fatal (missing tool)
#   2 = refuse-to-run (invalid/missing fields)
#   3 = shape-missing (edge_case_seeds enforcement failure)

# Path computation:
#   This file lives at: skills/autospec-test/tests/unit/v2/contract-loader-v2.bats
#   v2/ → unit/ → tests/ → autospec-test/ = 3 levels up for SKILL_DIR
#   autospec-test/ → skills/ → repo_root/  = 2 levels up for REPO_ROOT
SKILL_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../.." && pwd)"
REPO_ROOT="$(cd "$SKILL_DIR/../.." && pwd)"
SCHEMA="$REPO_ROOT/schemas/autospec-test-contract.schema.json"
VALIDATE="$SKILL_DIR/scripts/validate-contract.sh"
FIXTURES_V1="$SKILL_DIR/tests/fixtures/contracts"
FIXTURES_V2="$SKILL_DIR/tests/fixtures/contracts/v2"

# Helper: convert YAML fixture to JSON temp file for ajv (macOS-compatible mktemp)
yaml_to_json_tmp() {
  local yaml_file="$1"
  local tmp
  tmp=$(mktemp -t autospec-v2-fixture)
  yq -o=json '.' "$yaml_file" > "$tmp"
  echo "$tmp"
}

# ── Preconditions ──────────────────────────────────────────────────────────────

@test "ajv CLI is available" {
  command -v ajv
}

@test "yq CLI is available" {
  command -v yq
}

@test "jq CLI is available" {
  command -v jq
}

@test "schema file exists" {
  [ -f "$SCHEMA" ]
}

@test "validate-contract.sh exists and is executable" {
  [ -x "$VALIDATE" ]
}

# ── v2 fixtures exist ──────────────────────────────────────────────────────────

@test "v2 fixture: minimal-v2.yml exists" {
  [ -f "$FIXTURES_V2/minimal-v2.yml" ]
}

@test "v2 fixture: all-metrics.yml exists" {
  [ -f "$FIXTURES_V2/all-metrics.yml" ]
}

@test "v2 fixture: missing-edge-seeds.yml exists" {
  [ -f "$FIXTURES_V2/missing-edge-seeds.yml" ]
}

@test "v2 fixture: scoped-prod-with-v2.yml exists" {
  [ -f "$FIXTURES_V2/scoped-prod-with-v2.yml" ]
}

@test "v2 fixture: invalid-shapes.yml exists" {
  [ -f "$FIXTURES_V2/invalid-shapes.yml" ]
}

# ── Schema compiles (draft2020) ────────────────────────────────────────────────

@test "schema compiles with ajv draft2020" {
  run ajv compile -s "$SCHEMA" --spec=draft2020
  [ "$status" -eq 0 ]
  [[ "$output" == *"is valid"* ]]
}

# ── Schema accepts valid v2 fixtures (via validate-contract.sh) ───────────────
# Note: ajv validate --spec=draft2020 has a CLI flag-parsing quirk on this
# version of ajv-cli; validate-contract.sh handles it correctly via its own
# tempfile approach. These tests use validate-contract.sh as the authoritative
# accept/reject gate.

@test "schema accepts minimal-v2.yml" {
  local tmp
  tmp=$(yaml_to_json_tmp "$FIXTURES_V2/minimal-v2.yml")
  run bash "$VALIDATE" "$tmp"
  rm -f "$tmp"
  [ "$status" -eq 0 ]
}

@test "schema accepts all-metrics.yml" {
  local tmp
  tmp=$(yaml_to_json_tmp "$FIXTURES_V2/all-metrics.yml")
  run bash "$VALIDATE" "$tmp"
  rm -f "$tmp"
  [ "$status" -eq 0 ]
}

@test "schema accepts scoped-prod-with-v2.yml" {
  local tmp
  tmp=$(yaml_to_json_tmp "$FIXTURES_V2/scoped-prod-with-v2.yml")
  run bash "$VALIDATE" "$tmp"
  rm -f "$tmp"
  [ "$status" -eq 0 ]
}

# ── Schema parses invalid-shapes.yml (rejection is at validate-contract.sh level) ─

@test "invalid-shapes.yml is parseable YAML" {
  run yq -o=json '.' "$FIXTURES_V2/invalid-shapes.yml"
  [ "$status" -eq 0 ]
}

# ── validate-contract.sh cross-field rules ────────────────────────────────────

@test "missing-edge-seeds.yml rejects with exit 3 (shape-missing)" {
  local tmp
  tmp=$(yaml_to_json_tmp "$FIXTURES_V2/missing-edge-seeds.yml")
  run bash "$VALIDATE" "$tmp"
  rm -f "$tmp"
  [ "$status" -eq 3 ]
}

@test "missing-edge-seeds.yml stderr mentions shape" {
  local tmp
  tmp=$(yaml_to_json_tmp "$FIXTURES_V2/missing-edge-seeds.yml")
  run bash "$VALIDATE" "$tmp"
  rm -f "$tmp"
  [[ "$output" == *"shape"* ]]
}

@test "invalid-shapes.yml rejects with exit 3 (no shapes under enforcement)" {
  local tmp
  tmp=$(yaml_to_json_tmp "$FIXTURES_V2/invalid-shapes.yml")
  run bash "$VALIDATE" "$tmp"
  rm -f "$tmp"
  [ "$status" -eq 3 ]
}

@test "v2 enabled=true with no metrics rejects with exit 2 (enabled-requires-metrics)" {
  local tmp
  tmp=$(mktemp -t autospec-v2-no-metrics)
  cat > "$tmp" <<'EOF'
{
  "mode": "strict_isolation",
  "e2e": {
    "forbidden_url_patterns_intentionally_empty": true,
    "invariants_v2": {
      "enabled": true,
      "invariants": [],
      "thresholds": {
        "invariants_required_pass_rate": 100
      }
    }
  }
}
EOF
  run bash "$VALIDATE" "$tmp"
  rm -f "$tmp"
  [ "$status" -eq 2 ]
}

@test "apply_on_routes without leading slash rejects with exit 2" {
  local tmp
  tmp=$(mktemp -t autospec-v2-no-slash)
  cat > "$tmp" <<'EOF'
{
  "mode": "strict_isolation",
  "e2e": {
    "forbidden_url_patterns_intentionally_empty": true,
    "invariants_v2": {
      "enabled": true,
      "invariants": [
        {
          "id": "test-invariant",
          "kind": "every_visible_X_is_Y",
          "visible": "[data-testid=item]",
          "action": "role=button[name=/edit/i]",
          "apply_on_routes": ["dashboard"]
        }
      ],
      "thresholds": {
        "invariants_required_pass_rate": 100
      }
    }
  }
}
EOF
  run bash "$VALIDATE" "$tmp"
  rm -f "$tmp"
  [ "$status" -eq 2 ]
}

# ── v1 backward compatibility ─────────────────────────────────────────────────

@test "v1 minimal-valid.yml still passes validate-contract.sh" {
  local tmp
  tmp=$(yaml_to_json_tmp "$FIXTURES_V1/minimal-valid.yml")
  run bash "$VALIDATE" "$tmp"
  rm -f "$tmp"
  [ "$status" -eq 0 ]
}

@test "v1 mode-ii-valid.yml still passes validate-contract.sh" {
  local tmp
  tmp=$(yaml_to_json_tmp "$FIXTURES_V1/mode-ii-valid.yml")
  run bash "$VALIDATE" "$tmp"
  rm -f "$tmp"
  [ "$status" -eq 0 ]
}

@test "invariants_v2 field appears in schema" {
  grep -q 'invariants_v2' "$SCHEMA"
}

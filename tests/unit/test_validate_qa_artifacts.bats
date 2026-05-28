#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-qa-artifact-validator-XXXXXX)"
  mkdir -p "$TEST_TMPDIR/repo/.autospec"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

@test "validate-qa-artifacts.sh passes with valid minimal JSON artifacts" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TEST_TMPDIR/repo/.autospec/proof-matrix.json" <<'JSON'
{
  "schema_version": 1,
  "freshness": {
    "repo_commit_sha": "0123456789abcdef0123456789abcdef01234567",
    "spec_file": "docs/specs/example.md",
    "spec_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "environment": "dev",
    "generated_at": "2026-05-28T12:00:00Z"
  },
  "requirements": [
    {
      "requirement_id": "REQ-1",
      "spec_reference": "docs/specs/example.md#req-1",
      "expected_behavior": "User sees a converted result.",
      "implementation_files": ["src/App.tsx"],
      "automated_tests": [{"path": "tests/app.spec.ts", "name": "converts glutamine"}],
      "live_evidence": [{"kind": "network", "summary": "GET /api/chemical/usage returned 200"}],
      "status": "PASS"
    }
  ]
}
JSON

  cat > "$TEST_TMPDIR/repo/.autospec/control-intent-ledger.json" <<'JSON'
{
  "schema_version": 1,
  "freshness": {
    "repo_commit_sha": "0123456789abcdef0123456789abcdef01234567",
    "spec_file": "docs/specs/example.md",
    "spec_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "environment": "dev",
    "generated_at": "2026-05-28T12:00:00Z"
  },
  "status": "PASS",
  "controls": [
    {
      "control_id": "convert-button",
      "selector": "button[name=convert]",
      "label": "Convert",
      "control_type": "button",
      "classification": "functional",
      "intent": "Runs conversion",
      "spec_reference": "docs/specs/example.md#convert",
      "expected_effect": "Result panel updates",
      "tests": [{"path": "tests/app.spec.ts", "name": "converts glutamine"}],
      "evidence": [{"kind": "network", "summary": "Request returned a domain result"}]
    }
  ]
}
JSON

  cat > "$TEST_TMPDIR/repo/.autospec/mutation-proof.json" <<'JSON'
{
  "schema_version": 1,
  "freshness": {
    "repo_commit_sha": "0123456789abcdef0123456789abcdef01234567",
    "spec_file": "docs/specs/example.md",
    "spec_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "environment": "dev",
    "generated_at": "2026-05-28T12:00:00Z"
  },
  "mutations": [
    {
      "workflow_id": "convert",
      "mutation_type": "request_payload",
      "test": {"path": "tests/app.spec.ts", "name": "converts glutamine"},
      "expected_failure": "Selected target is missing from request",
      "observed_result": "failed_as_expected"
    }
  ]
}
JSON

  cat > "$TEST_TMPDIR/repo/.autospec/canary-results.json" <<'JSON'
{
  "schema_version": 1,
  "freshness": {
    "repo_commit_sha": "0123456789abcdef0123456789abcdef01234567",
    "spec_file": "docs/specs/example.md",
    "spec_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "environment": "dev",
    "generated_at": "2026-05-28T12:00:00Z"
  },
  "canaries": [
    {
      "workflow_id": "convert",
      "url": "https://dev.example.test",
      "representative_input": "glutamine",
      "expected_result": "Converted identifier visible",
      "result": "PASS",
      "evidence": [{"kind": "network", "summary": "No-mock smoke returned 200"}]
    }
  ]
}
JSON

  cat > "$TEST_TMPDIR/repo/.autospec/reliability.json" <<'JSON'
{
  "schema_version": 1,
  "freshness": {
    "repo_commit_sha": "0123456789abcdef0123456789abcdef01234567",
    "spec_file": "docs/specs/example.md",
    "spec_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "environment": "dev",
    "generated_at": "2026-05-28T12:00:00Z"
  },
  "workflows": [
    {
      "id": "convert",
      "name": "Convert identifier",
      "representative_inputs": ["glutamine"],
      "mock_policy": "mock_plus_no_mock",
      "no_mock_required": false,
      "expected_result": "Converted identifier visible"
    }
  ],
  "forbidden_false_green": ["dom_only_pass", "request_only_pass", "mocked_only_pass"]
}
JSON

  run bash "$REPO_ROOT/scripts/validate-qa-artifacts.sh" --repo "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
}

@test "validate-qa-artifacts.sh rejects malformed present artifacts" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  echo '{"schema_version":1,"requirements":[]}' > "$TEST_TMPDIR/repo/.autospec/proof-matrix.json"

  run bash "$REPO_ROOT/scripts/validate-qa-artifacts.sh" --repo "$TEST_TMPDIR/repo"
  [ "$status" -ne 0 ]
}

@test "reliability schema rejects pending deprecated surfaces without sunset issue" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TEST_TMPDIR/reliability-bad.json" <<'JSON'
{
  "schema_version": 1,
  "freshness": {
    "repo_commit_sha": "0123456789abcdef0123456789abcdef01234567",
    "spec_file": "docs/specs/example.md",
    "spec_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "environment": "dev",
    "generated_at": "2026-05-28T12:00:00Z"
  },
  "workflows": [
    {
      "id": "taxonomy",
      "name": "Taxonomy lookup",
      "representative_inputs": ["glutamine"],
      "mock_policy": "mock_plus_no_mock",
      "no_mock_required": false,
      "expected_result": "Postgres taxonomy result visible"
    }
  ],
  "forbidden_false_green": ["dom_only_pass"],
  "deprecated_surfaces": [
    {
      "id": "s3-taxonomy-cache",
      "kind": "bucket",
      "replacement": "Postgres chemical API store",
      "forbidden_action": "Do not upload cache documents to make smoke pass",
      "cleanup_status": "pending"
    }
  ]
}
JSON

  run ajv validate -s "$REPO_ROOT/schemas/autospec-reliability.schema.json" --spec=draft2020 -d "$TEST_TMPDIR/reliability-bad.json"
  [ "$status" -ne 0 ]
}

@test "validate-qa-artifacts.sh rejects a missing repo directory" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  run bash "$REPO_ROOT/scripts/validate-qa-artifacts.sh" --repo "$TEST_TMPDIR/missing"
  [ "$status" -eq 2 ]
  [[ "$output" == *"repo directory not found"* ]]
}

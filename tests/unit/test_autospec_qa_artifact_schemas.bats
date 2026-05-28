#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TMPDIR_TEST="$(mktemp -d /tmp/autospec-qa-schemas-XXXXXX)"
}

teardown() {
  rm -rf "$TMPDIR_TEST"
}

@test "autospec QA artifact schemas exist" {
  for schema in \
    autospec-proof-matrix.schema.json \
    autospec-reliability.schema.json \
    autospec-control-intent-ledger.schema.json \
    autospec-mutation-proof.schema.json \
    autospec-canary-results.schema.json; do
    [ -f "$REPO_ROOT/schemas/$schema" ]
  done
}

@test "autospec QA artifact schemas compile with ajv" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  for schema in \
    autospec-proof-matrix.schema.json \
    autospec-reliability.schema.json \
    autospec-control-intent-ledger.schema.json \
    autospec-mutation-proof.schema.json \
    autospec-canary-results.schema.json; do
    run ajv compile -s "$REPO_ROOT/schemas/$schema" --spec=draft2020
    [ "$status" -eq 0 ]
  done
}

@test "proof matrix schema accepts minimal evidence-bearing artifact" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TMPDIR_TEST/proof-matrix.json" <<'JSON'
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

  run ajv validate -s "$REPO_ROOT/schemas/autospec-proof-matrix.schema.json" --spec=draft2020 -d "$TMPDIR_TEST/proof-matrix.json"
  [ "$status" -eq 0 ]
}

@test "control ledger schema rejects unclassified controls in passable artifacts" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TMPDIR_TEST/control-ledger.json" <<'JSON'
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
      "classification": "unclassified",
      "intent": "Runs conversion",
      "spec_reference": "docs/specs/example.md#convert",
      "expected_effect": "Result panel updates",
      "evidence": [],
      "follow_up": "Classify the control before release"
    }
  ]
}
JSON

  run ajv validate -s "$REPO_ROOT/schemas/autospec-control-intent-ledger.schema.json" --spec=draft2020 -d "$TMPDIR_TEST/control-ledger.json"
  [ "$status" -ne 0 ]
}

@test "control ledger schema accepts unclassified controls in partial artifacts with follow-up" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TMPDIR_TEST/control-ledger-partial.json" <<'JSON'
{
  "schema_version": 1,
  "freshness": {
    "repo_commit_sha": "0123456789abcdef0123456789abcdef01234567",
    "spec_file": "docs/specs/example.md",
    "spec_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "environment": "dev",
    "generated_at": "2026-05-28T12:00:00Z"
  },
  "status": "PARTIAL",
  "controls": [
    {
      "control_id": "convert-button",
      "selector": "button[name=convert]",
      "label": "Convert",
      "control_type": "button",
      "classification": "unclassified",
      "intent": "Runs conversion",
      "spec_reference": "docs/specs/example.md#convert",
      "expected_effect": "Result panel updates",
      "evidence": [],
      "follow_up": "Classify the control before release"
    }
  ]
}
JSON

  run ajv validate -s "$REPO_ROOT/schemas/autospec-control-intent-ledger.schema.json" --spec=draft2020 -d "$TMPDIR_TEST/control-ledger-partial.json"
  [ "$status" -eq 0 ]
}

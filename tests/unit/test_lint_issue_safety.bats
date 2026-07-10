#!/usr/bin/env bats

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/scripts/lint-issue-safety.sh"
    FIX="$REPO_ROOT/tests/fixtures/issue-safety"
}

@test "lint-issue-safety: safe docs change passes" {
    run bash "$LINT" --title "Update config docs" "$FIX/safe-docs-change.md"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "SAFETY_PASS"
}

@test "lint-issue-safety: production deletion blocks" {
    run bash "$LINT" --title "Delete production data" "$FIX/malicious-production-delete.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "production-data-destruction"
}

@test "lint-issue-safety: vague data cleanup quarantines as ambiguous" {
    run bash "$LINT" --title "Clean old data" "$FIX/ambiguous-clean-data.md"
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "SAFETY_AMBIGUOUS"
    echo "$output" | grep -q "vague-data-cleanup"
}

@test "lint-issue-safety: trusted actor can reset test database" {
    run bash "$LINT" --actor berlinguyinca --title "Reset test database" "$FIX/trusted-test-db-reset.md"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "SAFETY_PASS"
    echo "$output" | grep -q "trusted:test_data_reset"
}

@test "lint-issue-safety: trusted actor cannot dump secrets" {
    run bash "$LINT" --actor berlinguyinca --title "Dump secrets" "$FIX/trusted-secret-dump.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "secret-exfiltration"
}

@test "lint-issue-safety: credential printing blocks" {
    run bash "$LINT" --title "Print repository credentials" "$FIX/credential-printing.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "credential-printing"
}

@test "lint-issue-safety: disabling ci checks blocks" {
    run bash "$LINT" --title "Disable CI checks" "$FIX/ci-disable.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "ci-or-review-bypass"
}

@test "lint-issue-safety: explicit auth backdoor blocks" {
    run bash "$LINT" --title "Add auth backdoor" "$FIX/auth-backdoor.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "auth-backdoor"
}

@test "lint-issue-safety: trusted test reset does not wipe unrelated backdoor finding" {
    run bash "$LINT" --actor berlinguyinca --title "Reset test database and add backdoor" "$FIX/trusted-reset-with-backdoor.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "auth-backdoor"
    ! echo "$output" | grep -q "trusted:test_data_reset"
}

@test "lint-issue-safety: duplicate config defaults emit one finding" {
    run bash "$LINT" --config "$FIX/duplicate-default-policy.yml" --title "Delete production data" "$FIX/malicious-production-delete.md"
    [ "$status" -eq 2 ]
    count="$(echo "$output" | grep -c "production-data-destruction")"
    [ "$count" -eq 1 ]
}

@test "lint-issue-safety: invalid YAML falls back to defaults and blocks dangerous body" {
    run bash "$LINT" --config "$FIX/invalid-policy.yml" --title "Delete production data" "$FIX/malicious-production-delete.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "production-data-destruction"
}

@test "lint-issue-safety: empty policy lists still preserve built-in production deletion block" {
    run bash "$LINT" --config "$FIX/weakening-policy.yml" --title "Delete production data" "$FIX/malicious-production-delete.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "production-data-destruction"
}

@test "lint-issue-safety: json mode emits decision field" {
    run bash "$LINT" --json --title "Clean old data" "$FIX/ambiguous-clean-data.md"
    [ "$status" -eq 1 ]
    echo "$output" | grep -q '"decision":"SAFETY_AMBIGUOUS"'
    echo "$output" | grep -q '"rule_id":"vague-data-cleanup"'
}

@test "autospec config schema accepts issue_intent_gate policy" {
    run python3 - "$REPO_ROOT/.autospec/autospec.yml" "$REPO_ROOT/schemas/autospec-config.schema.json" <<'PY'
import json
import sys
try:
    import yaml
    import jsonschema
except Exception as exc:
    print(f"missing optional validator module: {exc}")
    raise SystemExit(0)
config_path, schema_path = sys.argv[1], sys.argv[2]
with open(config_path, "r", encoding="utf-8") as fh:
    doc = yaml.safe_load(fh)
assert "issue_intent_gate" in doc["safety"], "missing safety.issue_intent_gate default"
with open(schema_path, "r", encoding="utf-8") as fh:
    schema = json.load(fh)
jsonschema.validate(doc, schema)
PY
    [ "$status" -eq 0 ]
}

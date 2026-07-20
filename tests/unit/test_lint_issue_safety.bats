#!/usr/bin/env bats

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    AUTOSPEC="$REPO_ROOT/target/debug/autospec"
    if [ ! -x "$AUTOSPEC" ]; then
        cargo build --quiet --manifest-path "$REPO_ROOT/Cargo.toml" -p autospec-cli --bin autospec
    fi
    FIX="$REPO_ROOT/tests/fixtures/issue-safety"
}

safety_lint() {
    "$AUTOSPEC" lint issue safety "$@"
}

@test "lint-issue-safety: safe docs change passes" {
    run safety_lint --title "Update config docs" "$FIX/safe-docs-change.md"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "SAFETY_PASS"
}

@test "lint-issue-safety: production deletion blocks" {
    run safety_lint --title "Delete production data" "$FIX/malicious-production-delete.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "production-data-destruction"
}

@test "lint-issue-safety: vague data cleanup quarantines as ambiguous" {
    run safety_lint --title "Clean old data" "$FIX/ambiguous-clean-data.md"
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "SAFETY_AMBIGUOUS"
    echo "$output" | grep -q "vague-data-cleanup"
}

@test "lint-issue-safety: trusted actor can reset test database" {
    run safety_lint --actor berlinguyinca --title "Reset test database" "$FIX/trusted-test-db-reset.md"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "SAFETY_PASS"
    echo "$output" | grep -q "trusted:test_data_reset"
}

@test "lint-issue-safety: trusted actor cannot dump secrets" {
    run safety_lint --actor berlinguyinca --title "Dump secrets" "$FIX/trusted-secret-dump.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "secret-exfiltration"
}

@test "lint-issue-safety: credential printing blocks" {
    run safety_lint --title "Print repository credentials" "$FIX/credential-printing.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "credential-printing"
}

@test "lint-issue-safety: disabling ci checks blocks" {
    run safety_lint --title "Disable CI checks" "$FIX/ci-disable.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "ci-or-review-bypass"
}

@test "lint-issue-safety: passive CI-bypass phrasing still blocks" {
    # "CI checks were disabled … review requirement removed" — real bypass in
    # passive/noun-first voice must stay caught by the past-participle arm.
    run safety_lint --title "Speed up merges" "$FIX/ci-passive-bypass.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "ci-or-review-bypass"
}

@test "lint-issue-safety: active noun-first bypass with object pronoun blocks" {
    # "disable it … remove them" (CI-noun is the verb's object, active voice) —
    # must stay caught; guards the past-participle-only regression.
    run safety_lint --title "Speed up merges" "$FIX/ci-active-pronoun-bypass.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "ci-or-review-bypass"
}

@test "lint-issue-safety: long single-sentence CI-bypass still blocks" {
    # verb→noun with a >40-char but same-sentence gap must stay caught; guards
    # the too-tight gap-cap regression.
    run safety_lint --title "Loosen protection" "$FIX/ci-longgap-bypass.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "ci-or-review-bypass"
}

@test "lint-issue-safety: descriptive CI prose does not false-positive (issue #1799)" {
    # Benign bug-description "…pending CI and skips every drain cycle" plus a
    # later "## Tests" heading must NOT match ci-or-review-bypass. Guards the
    # unbounded-`.*` cross-clause/cross-section false positive that stranded #1463.
    run safety_lint --title "fix: prevent conductor idle loop" "$FIX/ci-descriptive-benign.md"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "SAFETY_PASS"
}

@test "lint-issue-safety: explicit auth backdoor blocks" {
    run safety_lint --title "Add auth backdoor" "$FIX/auth-backdoor.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "auth-backdoor"
}

@test "lint-issue-safety: trusted test reset does not wipe unrelated backdoor finding" {
    run safety_lint --actor berlinguyinca --title "Reset test database and add backdoor" "$FIX/trusted-reset-with-backdoor.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "auth-backdoor"
    ! echo "$output" | grep -q "trusted:test_data_reset"
}

@test "lint-issue-safety: duplicate config defaults emit one finding" {
    run safety_lint --config "$FIX/duplicate-default-policy.yml" --title "Delete production data" "$FIX/malicious-production-delete.md"
    [ "$status" -eq 2 ]
    count="$(echo "$output" | grep -c "production-data-destruction")"
    [ "$count" -eq 1 ]
}

@test "lint-issue-safety: malformed explicit YAML fails closed before evaluation" {
    run safety_lint --config "$FIX/invalid-policy.yml" --title "Delete production data" "$FIX/malicious-production-delete.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "could not parse issue safety policy"
    ! echo "$output" | grep -Eq "SAFETY_(PASS|AMBIGUOUS|BLOCK)"
    ! echo "$output" | grep -q "production-data-destruction"
}

@test "lint-issue-safety: empty policy lists still preserve built-in production deletion block" {
    run safety_lint --config "$FIX/weakening-policy.yml" --title "Delete production data" "$FIX/malicious-production-delete.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "production-data-destruction"
}

@test "lint-issue-safety: an unsupported custom regex fails closed" {
    policy="$BATS_TEST_TMPDIR/custom-safety-policy.yml"
    cat > "$policy" <<'EOF'
safety:
  issue_intent_gate:
    block_patterns:
      - id: company-secret-policy
        patterns:
          - "(?i)company secret"
EOF

    run safety_lint --config "$policy" --title "Update docs" "$FIX/safe-docs-change.md"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "SAFETY_BLOCK"
    echo "$output" | grep -q "invalid-policy-regex"
}

@test "lint-issue-safety: configured trusted actor can perform scoped test reset" {
    policy="$BATS_TEST_TMPDIR/trusted-actor-policy.yml"
    cat > "$policy" <<'EOF'
safety:
  issue_intent_gate:
    trusted_actors:
      - login: release-operator
EOF

    run safety_lint --config "$policy" --actor release-operator --title "Reset test database" "$FIX/trusted-test-db-reset.md"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "SAFETY_PASS"
    echo "$output" | grep -q "trusted:test_data_reset"
}

@test "lint-issue-safety: json mode emits decision field" {
    run safety_lint --json --title "Clean old data" "$FIX/ambiguous-clean-data.md"
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

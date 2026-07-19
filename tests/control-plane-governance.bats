#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
SCRIPT="$REPO_ROOT/scripts/autospec-control-plane.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  OUTPUT="$TEST_TMP/governance-dry-run.txt"
  bash "$SCRIPT" bootstrap --dry-run --owner berlinguyinca --governance-repo autospec-governance > "$OUTPUT"
}

teardown() {
  rm -rf "$TEST_TMP"
}

assert_contains() {
  local needle="$1"
  grep -Fq -- "$needle" "$OUTPUT" || {
    printf 'missing expected text: %s\n' "$needle" >&2
    printf '%s\n' '--- dry-run output ---' >&2
    cat "$OUTPUT" >&2
    return 1
  }
}

@test "dry-run emits policy and rule JSON schemas with required governance fields" {
  assert_contains "--- autospec-governance/schemas/policy.schema.json ---"
  assert_contains '"policy_id"'
  assert_contains '"project_class"'
  assert_contains '"privacy_tier"'
  assert_contains '"priority_waterfall"'
  assert_contains '"merge_rules"'
  assert_contains '"cost_limits"'
  assert_contains '"evidence_requirements"'

  assert_contains "--- autospec-governance/schemas/rule.schema.json ---"
  assert_contains '"rule_id"'
  assert_contains '"category"'
  assert_contains '"severity"'
  assert_contains '"deterministic_checks"'
}

@test "dry-run renders policy packs for all six project classifications" {
  for class in open-source private-personal private-company client-project research sandbox; do
    assert_contains "project_class: $class"
  done

  assert_contains "policies/open-source-maintainer-default.yml"
  assert_contains "policies/private-personal-default.yml"
  assert_contains "policies/private-company-default.yml"
  assert_contains "policies/client-project-default.yml"
  assert_contains "policies/research-default.yml"
  assert_contains "policies/sandbox-default.yml"
}

@test "dry-run renders project fixtures for all six classifications" {
  assert_contains "fixtures/projects/open-source-cli.yml"
  assert_contains "fixtures/projects/private-personal-app.yml"
  assert_contains "fixtures/projects/private-company-saas.yml"
  assert_contains "fixtures/projects/client-webapp.yml"
  assert_contains "fixtures/projects/research-notebook.yml"
  assert_contains "fixtures/projects/sandbox-lab.yml"
}

@test "dry-run policy packs expose priority privacy merge cost and evidence rules" {
  assert_contains "priority_waterfall:"
  assert_contains "privacy_tier: metadata-only"
  assert_contains "privacy_tier: summary"
  assert_contains "privacy_tier: evidence"
  assert_contains "raw_logs_allowed: false"
  assert_contains "raw_logs_allowed: true"
  assert_contains "merge_rules:"
  assert_contains "require_ci_green: true"
  assert_contains "cost_limits:"
  assert_contains "daily_usd:"
  assert_contains "evidence_requirements:"
  assert_contains "runtime_proof_required: true"
}

@test "dry-run rule catalogs cover required deterministic governance categories" {
  for rule in qa testing documentation security accessibility performance skill-generation release-readiness; do
    assert_contains "rules/$rule.yml"
  done
  assert_contains "category: priority"
  assert_contains "category: privacy"
  assert_contains "category: merge"
  assert_contains "category: cost"
  assert_contains "category: evidence"
}

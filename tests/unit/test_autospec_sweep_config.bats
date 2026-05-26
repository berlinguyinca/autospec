#!/usr/bin/env bats
# tests/unit/test_autospec_sweep_config.bats — first-run autospec config wizard.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  WIZARD="$REPO_ROOT/skills/autospec-sweep/scripts/wizard.sh"
  SCHEMA="$REPO_ROOT/schemas/autospec-config.schema.json"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-sweep-config-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

@test "autospec-sweep init writes a tracked top-level config with all steps enabled by default" {
  mkdir -p "$TEST_TMPDIR/repo"

  run bash "$WIZARD" init --repo-root "$TEST_TMPDIR/repo" --answers "$REPO_ROOT/tests/fixtures/autospec-sweep/minimal-answers.yml"

  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/autospec.yml" ]

  run yq -r '.version' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [ "$output" = "1" ]

  run yq -r '.steps.define.enabled' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [ "$output" = "true" ]

  run yq -r '.steps.run.enabled' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [ "$output" = "true" ]

  run yq -r '.steps.sweep.enabled' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [ "$output" = "true" ]

  run yq -r '.sweep.spec_sync.enabled' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [ "$output" = "true" ]

  run yq -r '.continuous_improvement.docs.enabled' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [ "$output" = "true" ]

  run yq -r '.continuous_improvement.tests.enabled' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [ "$output" = "true" ]

  run yq -r '.continuous_improvement.code.enabled' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [ "$output" = "true" ]

  run yq -r '.execution.tests.run_all_every_sweep' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [ "$output" = "true" ]

  run yq -r '.execution.deployment.deploy_if_tests_require' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [ "$output" = "true" ]
}

@test "autospec config schema compiles and accepts generated defaults" {
  mkdir -p "$TEST_TMPDIR/repo"
  bash "$WIZARD" init --repo-root "$TEST_TMPDIR/repo" --answers "$REPO_ROOT/tests/fixtures/autospec-sweep/minimal-answers.yml"

  run ajv compile -s "$SCHEMA" --spec=draft2020
  [ "$status" -eq 0 ]

  yq -o=json '.' "$TEST_TMPDIR/repo/.autospec/autospec.yml" > "$TEST_TMPDIR/config.json"
  run ajv validate -s "$SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/config.json"
  [ "$status" -eq 0 ]
}

@test "autospec-sweep init refuses to overwrite an existing config without --force" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec"
  printf 'version: 1\n' > "$TEST_TMPDIR/repo/.autospec/autospec.yml"

  run bash "$WIZARD" init --repo-root "$TEST_TMPDIR/repo" --answers "$REPO_ROOT/tests/fixtures/autospec-sweep/minimal-answers.yml"

  [ "$status" -eq 2 ]
  [[ "$output" == *"already exists"* ]]
  run yq -r '.steps.define.enabled // "missing"' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [ "$output" = "missing" ]
}

@test "autospec-sweep init refuses when tracked config path is gitignored" {
  mkdir -p "$TEST_TMPDIR/repo"
  git -C "$TEST_TMPDIR/repo" init -q
  printf '.autospec/\n' > "$TEST_TMPDIR/repo/.gitignore"

  run bash "$WIZARD" init --repo-root "$TEST_TMPDIR/repo" --answers "$REPO_ROOT/tests/fixtures/autospec-sweep/minimal-answers.yml"

  [ "$status" -eq 2 ]
  [[ "$output" == *"ignored by git"* ]]
}

@test "autospec-sweep init validates boolean and safety answers" {
  mkdir -p "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/bad-answers.yml" <<'YAML'
profile: full
safety: unsafe_prod
team: auto
allow_competitor_research: maybe
YAML

  run bash "$WIZARD" init --repo-root "$TEST_TMPDIR/repo" --answers "$TEST_TMPDIR/bad-answers.yml"

  [ "$status" -eq 2 ]
  [[ "$output" == *"safety must be"* ]]
}

@test "autospec-sweep init records project-specific findings and follow-up questions" {
  mkdir -p "$TEST_TMPDIR/repo"
  printf '{"scripts":{"test":"vitest","e2e":"playwright test"}}\n' > "$TEST_TMPDIR/repo/package.json"
  printf 'export default { use: { baseURL: "http://localhost:3000" } };\n' > "$TEST_TMPDIR/repo/playwright.config.ts"

  run bash "$WIZARD" init --repo-root "$TEST_TMPDIR/repo" --answers "$REPO_ROOT/tests/fixtures/autospec-sweep/minimal-answers.yml"

  [ "$status" -eq 0 ]
  run yq -r '.project.findings.stack[]' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [[ "$output" == *"node"* ]]
  [[ "$output" == *"playwright"* ]]

  run yq -r '.project.questions[]' "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  [[ "$output" == *"base URL"* ]]
}

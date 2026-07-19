#!/usr/bin/env bats
# tests/unit/test_constitution_validation.bats — local Constitution/Baseline validation.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SCRIPT="$REPO_ROOT/scripts/autospec-constitution-validate.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-constitution-validation-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_valid_constitution() {
  local root="$1"
  mkdir -p "$root/doctrine" "$root/schemas"
  printf '# Constitution\n' > "$root/README.md"
  printf '# Vision\n' > "$root/VISION.md"
  printf '# Law\n' > "$root/CONSTITUTION.md"
  printf '# Doctrine\n' > "$root/doctrine/quality.md"
  printf '{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object"}\n' \
    > "$root/schemas/constitution.schema.json"
}

write_valid_baselines() {
  local root="$1"
  mkdir -p "$root/profiles/web" "$root/profiles/ai-platform" "$root/profiles/analytics" "$root/schemas"
  printf '# Web\n' > "$root/profiles/web/README.md"
  printf '# AI Platform\n' > "$root/profiles/ai-platform/README.md"
  printf '# Analytics\n' > "$root/profiles/analytics/README.md"
  printf '{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object"}\n' \
    > "$root/schemas/baseline-pack.schema.json"
}

write_config() {
  local repo="$1"
  local constitution_path="$2"
  local baselines_path="$3"
  mkdir -p "$repo/.autospec"
  cat > "$repo/.autospec/autospec.yml" <<YAML
version: 1
constitution:
  source: local
  path: $constitution_path
  version: 0.1.0
baselines:
  source: local
  path: $baselines_path
  profiles:
    - web
    - ai-platform
    - analytics
YAML
}

@test "valid local config writes pass JSON and Markdown reports" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_valid_constitution "$TEST_TMPDIR/autospec-constitution"
  write_valid_baselines "$TEST_TMPDIR/autospec-baselines"
  write_config "$TEST_TMPDIR/repo" "../autospec-constitution" "../autospec-baselines"

  run bash "$SCRIPT" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 0 ]
  [[ "$output" == *"constitution validation: PASS"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/constitution-validation.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/constitution-validation.md" ]
  run jq -r '.status' "$TEST_TMPDIR/repo/.autospec/reports/constitution-validation.json"
  [ "$output" = "pass" ]
  grep -q 'web' "$TEST_TMPDIR/repo/.autospec/reports/constitution-validation.md"
}

@test "missing constitution path fails with actionable error output" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_valid_baselines "$TEST_TMPDIR/autospec-baselines"
  write_config "$TEST_TMPDIR/repo" "../missing-constitution" "../autospec-baselines"

  run bash "$SCRIPT" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  [[ "$output" == *"constitution path does not exist"* ]]
  [[ "$output" == *"Create the directory or update constitution.path"* ]]
  run jq -r '.errors[0].code' "$TEST_TMPDIR/repo/.autospec/reports/constitution-validation.json"
  [ "$output" = "CONSTITUTION_PATH_MISSING" ]
}

@test "missing required constitution file is reported by filename" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_valid_constitution "$TEST_TMPDIR/autospec-constitution"
  rm "$TEST_TMPDIR/autospec-constitution/VISION.md"
  write_valid_baselines "$TEST_TMPDIR/autospec-baselines"
  write_config "$TEST_TMPDIR/repo" "../autospec-constitution" "../autospec-baselines"

  run bash "$SCRIPT" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  [[ "$output" == *"required constitution file is missing: VISION.md"* ]]
  expected_path="$(cd "$TEST_TMPDIR/autospec-constitution" && pwd -P)/VISION.md"
  run jq -r '.errors[] | select(.code=="CONSTITUTION_FILE_MISSING") | .path' \
    "$TEST_TMPDIR/repo/.autospec/reports/constitution-validation.json"
  [ "$output" = "$expected_path" ]
}

@test "missing requested baseline profile fails by profile id" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_valid_constitution "$TEST_TMPDIR/autospec-constitution"
  write_valid_baselines "$TEST_TMPDIR/autospec-baselines"
  rm -rf "$TEST_TMPDIR/autospec-baselines/profiles/analytics"
  write_config "$TEST_TMPDIR/repo" "../autospec-constitution" "../autospec-baselines"

  run bash "$SCRIPT" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  [[ "$output" == *"requested baseline profile is missing: analytics"* ]]
  run jq -r '.errors[] | select(.code=="BASELINE_PROFILE_MISSING") | .profile' \
    "$TEST_TMPDIR/repo/.autospec/reports/constitution-validation.json"
  [ "$output" = "analytics" ]
}

@test "malformed config fails with clear parse error and report" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec"
  cat > "$TEST_TMPDIR/repo/.autospec/autospec.yml" <<'YAML'
version: 1
constitution:
  source: [local
YAML

  run bash "$SCRIPT" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 2 ]
  [[ "$output" == *"failed to parse .autospec/autospec.yml"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/constitution-validation.json" ]
  run jq -r '.status' "$TEST_TMPDIR/repo/.autospec/reports/constitution-validation.json"
  [ "$output" = "error" ]
}

@test "remote source is rejected because this slice is local-only" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec"
  cat > "$TEST_TMPDIR/repo/.autospec/autospec.yml" <<'YAML'
version: 1
constitution:
  source: github
  path: berlinguyinca/autospec-constitution
baselines:
  source: local
  path: ../autospec-baselines
  profiles: []
YAML

  run bash "$SCRIPT" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  [[ "$output" == *"constitution.source must be local"* ]]
  [[ "$output" == *"GitHub sources are not supported by this validator"* ]]
}

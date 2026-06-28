#!/usr/bin/env bats
# tests/unit/test_baseline_composition.bats — local Baseline profile composition.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SCRIPT="$REPO_ROOT/scripts/autospec-baseline-compose.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-baseline-composition-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_config() {
  local repo="$1"
  local baselines_path="$2"
  shift 2
  mkdir -p "$repo/.autospec"
  cat > "$repo/.autospec/autospec.yml" <<YAML
version: 1
baselines:
  source: local
  path: $baselines_path
  profiles:
YAML
  for profile in "$@"; do
    printf '    - %s\n' "$profile" >> "$repo/.autospec/autospec.yml"
  done
}

write_profile() {
  local root="$1"
  local name="$2"
  local body="$3"
  mkdir -p "$root/profiles/$name"
  printf '%s\n' "$body" > "$root/profiles/$name/baseline.yml"
}

write_baseline_root() {
  local root="$1"
  mkdir -p "$root/schemas"
  printf '{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object"}\n' \
    > "$root/schemas/baseline-profile.schema.json"
}

@test "valid requested profiles compose into JSON and Markdown reports" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_baseline_root "$TEST_TMPDIR/autospec-baselines"
  write_profile "$TEST_TMPDIR/autospec-baselines" "ai-platform" 'id: ai-platform
capabilities:
  - id: model-routing
    description: Route work to model tiers.
requirements:
  - id: review-depth
    value: high'
  write_profile "$TEST_TMPDIR/autospec-baselines" "web" 'id: web
depends_on:
  - ai-platform
capabilities:
  - id: http-routing
    description: Validate HTTP route behavior.
requirements:
  - id: accessibility
    value: wcag-aa'
  write_config "$TEST_TMPDIR/repo" "../autospec-baselines" ai-platform web

  run bash "$SCRIPT" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 0 ]
  [[ "$output" == *"baseline composition: PASS"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.md" ]
  run jq -r '.status' "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.json"
  [ "$output" = "pass" ]
  run jq -r '.composed.capabilities[].id' "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.json"
  [[ "$output" == *"model-routing"* ]]
  [[ "$output" == *"http-routing"* ]]
  grep -q 'ai-platform, web' "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.md"
}

@test "missing requested profile is reported clearly" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_baseline_root "$TEST_TMPDIR/autospec-baselines"
  write_profile "$TEST_TMPDIR/autospec-baselines" "web" 'id: web
capabilities:
  - id: http-routing'
  write_config "$TEST_TMPDIR/repo" "../autospec-baselines" web analytics

  run bash "$SCRIPT" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  [[ "$output" == *"requested baseline profile is missing: analytics"* ]]
  run jq -r '.findings[] | select(.code=="BASELINE_PROFILE_MISSING") | .profile' \
    "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.json"
  [ "$output" = "analytics" ]
}

@test "duplicate capability ids across profiles are reported" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_baseline_root "$TEST_TMPDIR/autospec-baselines"
  write_profile "$TEST_TMPDIR/autospec-baselines" "web" 'id: web
capabilities:
  - id: telemetry'
  write_profile "$TEST_TMPDIR/autospec-baselines" "analytics" 'id: analytics
capabilities:
  - id: telemetry'
  write_config "$TEST_TMPDIR/repo" "../autospec-baselines" web analytics

  run bash "$SCRIPT" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  [[ "$output" == *"duplicate capability id: telemetry"* ]]
  run jq -r '.findings[] | select(.code=="DUPLICATE_CAPABILITY") | .capability' \
    "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.json"
  [ "$output" = "telemetry" ]
}

@test "conflicting requirement values are reported" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_baseline_root "$TEST_TMPDIR/autospec-baselines"
  write_profile "$TEST_TMPDIR/autospec-baselines" "web" 'id: web
requirements:
  - id: data-retention
    value: 30d'
  write_profile "$TEST_TMPDIR/autospec-baselines" "analytics" 'id: analytics
requirements:
  - id: data-retention
    value: 90d'
  write_config "$TEST_TMPDIR/repo" "../autospec-baselines" web analytics

  run bash "$SCRIPT" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  [[ "$output" == *"conflicting requirement value: data-retention"* ]]
  run jq -r '.findings[] | select(.code=="REQUIREMENT_CONFLICT") | .requirement' \
    "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.json"
  [ "$output" = "data-retention" ]
}

@test "missing dependencies and order problems are reported" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_baseline_root "$TEST_TMPDIR/autospec-baselines"
  write_profile "$TEST_TMPDIR/autospec-baselines" "web" 'id: web
depends_on:
  - ai-platform
  - security
capabilities:
  - id: http-routing'
  write_profile "$TEST_TMPDIR/autospec-baselines" "ai-platform" 'id: ai-platform
capabilities:
  - id: model-routing'
  write_config "$TEST_TMPDIR/repo" "../autospec-baselines" web ai-platform

  run bash "$SCRIPT" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  [[ "$output" == *"profile dependency is requested after dependent: web depends on ai-platform"* ]]
  [[ "$output" == *"profile dependency is missing: web depends on security"* ]]
}

@test "unsupported profile names are rejected before filesystem lookup" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_baseline_root "$TEST_TMPDIR/autospec-baselines"
  write_config "$TEST_TMPDIR/repo" "../autospec-baselines" "../escape"

  run bash "$SCRIPT" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  [[ "$output" == *"unsupported baseline profile name: ../escape"* ]]
  run jq -r '.findings[] | select(.code=="UNSUPPORTED_PROFILE_NAME") | .profile' \
    "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.json"
  [ "$output" = "../escape" ]
}

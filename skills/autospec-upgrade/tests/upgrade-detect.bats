#!/usr/bin/env bats
# upgrade-detect.bats — TDD suite for upgrade-detect.sh (issue #1173)
# No network access, no installs. Uses fixture repos under tests/fixtures/detect/.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/upgrade-detect.sh"
FIXTURES="${BATS_TEST_DIRNAME}/fixtures/detect"

# ── Existence / executability ─────────────────────────────────────────────────

@test "upgrade-detect.sh exists and is executable" {
  [ -x "$SCRIPT" ]
}

@test "upgrade-detect.sh emits single-line JSON (no newline in output)" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  # Output must be exactly one line
  line_count="$(printf '%s\n' "$output" | grep -c .)"
  [ "$line_count" -eq 1 ]
}

@test "upgrade-detect.sh output is valid JSON" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq . > /dev/null
}

# ── npm + Angular + karma fixture ─────────────────────────────────────────────

@test "npm+angular: frameworks contains angular" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.frameworks | index("angular") != null'
}

@test "npm+angular: versions.angular is set" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  ver="$(printf '%s\n' "$output" | jq -r '.versions.angular')"
  [ -n "$ver" ]
  [ "$ver" != "null" ]
}

@test "npm+angular: versions.angular major is 17" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  ver="$(printf '%s\n' "$output" | jq -r '.versions.angular')"
  major="$(printf '%s\n' "$ver" | cut -d. -f1)"
  [ "$major" = "17" ]
}

@test "npm+angular: package_manager is npm" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  pm="$(printf '%s\n' "$output" | jq -r '.package_manager')"
  [ "$pm" = "npm" ]
}

@test "npm+angular: runners contains karma" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.runners | index("karma") != null'
}

@test "npm+angular: has_tests is true" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  has="$(printf '%s\n' "$output" | jq -r '.has_tests')"
  [ "$has" = "true" ]
}

@test "npm+angular: monorepo is false" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  mono="$(printf '%s\n' "$output" | jq -r '.monorepo')"
  [ "$mono" = "false" ]
}

# ── pnpm + Next.js fixture ─────────────────────────────────────────────────────

@test "pnpm+next: frameworks contains next" {
  run "$SCRIPT" --root "$FIXTURES/pnpm-next"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.frameworks | index("next") != null'
}

@test "pnpm+next: frameworks also contains react (next implies react)" {
  run "$SCRIPT" --root "$FIXTURES/pnpm-next"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.frameworks | index("react") != null'
}

@test "pnpm+next: versions.next major is 14" {
  run "$SCRIPT" --root "$FIXTURES/pnpm-next"
  [ "$status" -eq 0 ]
  ver="$(printf '%s\n' "$output" | jq -r '.versions.next')"
  major="$(printf '%s\n' "$ver" | cut -d. -f1)"
  [ "$major" = "14" ]
}

@test "pnpm+next: package_manager is pnpm" {
  run "$SCRIPT" --root "$FIXTURES/pnpm-next"
  [ "$status" -eq 0 ]
  pm="$(printf '%s\n' "$output" | jq -r '.package_manager')"
  [ "$pm" = "pnpm" ]
}

@test "pnpm+next: has_tests is true" {
  run "$SCRIPT" --root "$FIXTURES/pnpm-next"
  [ "$status" -eq 0 ]
  has="$(printf '%s\n' "$output" | jq -r '.has_tests')"
  [ "$has" = "true" ]
}

# ── yarn + React + jest fixture ───────────────────────────────────────────────

@test "yarn+react+jest: frameworks contains react" {
  run "$SCRIPT" --root "$FIXTURES/yarn-react-jest"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.frameworks | index("react") != null'
}

@test "yarn+react+jest: versions.react major is 18" {
  run "$SCRIPT" --root "$FIXTURES/yarn-react-jest"
  [ "$status" -eq 0 ]
  ver="$(printf '%s\n' "$output" | jq -r '.versions.react')"
  major="$(printf '%s\n' "$ver" | cut -d. -f1)"
  [ "$major" = "18" ]
}

@test "yarn+react+jest: package_manager is yarn" {
  run "$SCRIPT" --root "$FIXTURES/yarn-react-jest"
  [ "$status" -eq 0 ]
  pm="$(printf '%s\n' "$output" | jq -r '.package_manager')"
  [ "$pm" = "yarn" ]
}

@test "yarn+react+jest: runners contains jest" {
  run "$SCRIPT" --root "$FIXTURES/yarn-react-jest"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.runners | index("jest") != null'
}

@test "yarn+react+jest: has_tests is true" {
  run "$SCRIPT" --root "$FIXTURES/yarn-react-jest"
  [ "$status" -eq 0 ]
  has="$(printf '%s\n' "$output" | jq -r '.has_tests')"
  [ "$has" = "true" ]
}

# ── bun + React + vitest fixture ──────────────────────────────────────────────

@test "bun+react+vitest: frameworks contains react" {
  run "$SCRIPT" --root "$FIXTURES/bun-react-vitest"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.frameworks | index("react") != null'
}

@test "bun+react+vitest: package_manager is bun" {
  run "$SCRIPT" --root "$FIXTURES/bun-react-vitest"
  [ "$status" -eq 0 ]
  pm="$(printf '%s\n' "$output" | jq -r '.package_manager')"
  [ "$pm" = "bun" ]
}

@test "bun+react+vitest: runners contains vitest" {
  run "$SCRIPT" --root "$FIXTURES/bun-react-vitest"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.runners | index("vitest") != null'
}

@test "bun+react+vitest: has_tests is true" {
  run "$SCRIPT" --root "$FIXTURES/bun-react-vitest"
  [ "$status" -eq 0 ]
  has="$(printf '%s\n' "$output" | jq -r '.has_tests')"
  [ "$has" = "true" ]
}

# ── monorepo (nx.json) fixture ────────────────────────────────────────────────

@test "monorepo-nx: monorepo is true" {
  run "$SCRIPT" --root "$FIXTURES/monorepo-nx"
  [ "$status" -eq 0 ]
  mono="$(printf '%s\n' "$output" | jq -r '.monorepo')"
  [ "$mono" = "true" ]
}

@test "monorepo-nx: package_manager is npm (package-lock.json)" {
  run "$SCRIPT" --root "$FIXTURES/monorepo-nx"
  [ "$status" -eq 0 ]
  pm="$(printf '%s\n' "$output" | jq -r '.package_manager')"
  [ "$pm" = "npm" ]
}

# ── zero-tests fixture ────────────────────────────────────────────────────────

@test "zero-tests: has_tests is false" {
  run "$SCRIPT" --root "$FIXTURES/zero-tests"
  [ "$status" -eq 0 ]
  has="$(printf '%s\n' "$output" | jq -r '.has_tests')"
  [ "$has" = "false" ]
}

@test "zero-tests: frameworks contains angular" {
  run "$SCRIPT" --root "$FIXTURES/zero-tests"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.frameworks | index("angular") != null'
}

@test "zero-tests: package_manager is pnpm" {
  run "$SCRIPT" --root "$FIXTURES/zero-tests"
  [ "$status" -eq 0 ]
  pm="$(printf '%s\n' "$output" | jq -r '.package_manager')"
  [ "$pm" = "pnpm" ]
}

# ── unknown-stack fixture ─────────────────────────────────────────────────────

@test "unknown-stack: frameworks is [\"unknown\"]" {
  run "$SCRIPT" --root "$FIXTURES/unknown-stack"
  [ "$status" -eq 0 ]
  fw="$(printf '%s\n' "$output" | jq -r '.frameworks[0]')"
  [ "$fw" = "unknown" ]
  count="$(printf '%s\n' "$output" | jq '.frameworks | length')"
  [ "$count" = "1" ]
}

@test "unknown-stack: exit code is 0" {
  run "$SCRIPT" --root "$FIXTURES/unknown-stack"
  [ "$status" -eq 0 ]
}

@test "unknown-stack: package_manager is yarn" {
  run "$SCRIPT" --root "$FIXTURES/unknown-stack"
  [ "$status" -eq 0 ]
  pm="$(printf '%s\n' "$output" | jq -r '.package_manager')"
  [ "$pm" = "yarn" ]
}

# ── JSON schema completeness ──────────────────────────────────────────────────

@test "output JSON has all required top-level keys" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e 'has("frameworks") and has("versions") and has("package_manager") and has("runners") and has("monorepo") and has("has_tests")'
}

@test "frameworks is an array" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.frameworks | type == "array"'
}

@test "runners is an array" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.runners | type == "array"'
}

@test "versions is an object" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.versions | type == "object"'
}

@test "monorepo is a boolean" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.monorepo | type == "boolean"'
}

@test "has_tests is a boolean" {
  run "$SCRIPT" --root "$FIXTURES/npm-angular-karma"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.has_tests | type == "boolean"'
}

# ── Default root (no --root flag, uses cwd) ───────────────────────────────────

@test "upgrade-detect.sh defaults to cwd when no --root given" {
  cd "$FIXTURES/yarn-react-jest" && run "$SCRIPT"
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | jq -e '.frameworks | index("react") != null'
}

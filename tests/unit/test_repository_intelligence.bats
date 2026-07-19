#!/usr/bin/env bats
# tests/unit/test_repository_intelligence.bats — local repository intelligence reports.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  DISCOVER="$REPO_ROOT/scripts/autospec-discover-metadata.sh"
  BASELINE_GAP="$REPO_ROOT/scripts/autospec-baseline-gap.sh"
  CONSTITUTIONAL_GAP="$REPO_ROOT/scripts/autospec-constitutional-gap.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-repository-intelligence-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_autospec_config() {
  local repo="$1"
  mkdir -p "$repo/.autospec"
  cat > "$repo/.autospec/autospec.yml" <<'YAML'
version: 1
constitution:
  source: local
  path: ../autospec-constitution
baselines:
  source: local
  path: ../autospec-baselines
  profiles:
    - web
YAML
}

write_constitution() {
  local root="$1"
  mkdir -p "$root/doctrine" "$root/schemas"
  printf '# Constitution\n' > "$root/README.md"
  printf '# Vision\n' > "$root/VISION.md"
  printf '# Law\n' > "$root/CONSTITUTION.md"
  printf '# Testing Doctrine\n' > "$root/doctrine/testing.md"
  printf '{"type":"object"}\n' > "$root/schemas/constitution.schema.json"
}

write_composition() {
  local repo="$1"
  mkdir -p "$repo/.autospec/reports"
  cat > "$repo/.autospec/reports/baseline-composition.json" <<'JSON'
{
  "version": 1,
  "status": "pass",
  "composed": {
    "capabilities": [
      {"id": "documentation", "profile": "web"},
      {"id": "testing", "profile": "web"},
      {"id": "api", "profile": "web"},
      {"id": "ui", "profile": "web"},
      {"id": "playwright-e2e", "profile": "web"},
      {"id": "manual-review", "profile": "web", "opt_out": true},
      {"id": "unknown-control", "profile": "web"}
    ],
    "requirements": [
      {"id": "documentation", "value": "required", "profile": "web"},
      {"id": "testing", "value": "required", "profile": "web"}
    ],
    "dependencies": []
  },
  "findings": []
}
JSON
}

write_simple_repo() {
  local repo="$1"
  mkdir -p "$repo/docs" "$repo/src" "$repo/tests/unit" "$repo/.github/workflows"
  write_autospec_config "$repo"
  cat > "$repo/README.md" <<'MD'
# Sample Service

Sample Service processes customer events and exposes an HTTP API.
MD
  printf '# User Guide\n' > "$repo/docs/USER_MANUAL.md"
  printf 'console.log("build");\n' > "$repo/src/index.js"
  printf 'test("works", () => {});\n' > "$repo/tests/unit/sample.test.js"
  cat > "$repo/package.json" <<'JSON'
{
  "name": "sample-service",
  "scripts": {
    "build": "vite build",
    "test": "vitest run"
  },
  "dependencies": {
    "express": "^4.0.0"
  },
  "devDependencies": {
    "vitest": "^1.0.0"
  }
}
JSON
  printf 'name: ci\n' > "$repo/.github/workflows/ci.yml"
}

@test "metadata discovery writes state and report files with evidence and confidence" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_simple_repo "$TEST_TMPDIR/repo"

  run bash "$DISCOVER" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 0 ]
  [[ "$output" == *"metadata discovery: PASS"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/product-purpose.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/technology-registry.yml" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/repository-inventory.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/docs-coverage-map.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/test-coverage-map.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/api-surface.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/ui-surface.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/metadata-discovery.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/metadata-discovery.md" ]
  run jq -r '.facts.repo_name.value' "$TEST_TMPDIR/repo/.autospec/reports/metadata-discovery.json"
  [ "$output" = "repo" ]
  run jq -r '.facts.languages.value[]' "$TEST_TMPDIR/repo/.autospec/reports/metadata-discovery.json"
  [[ "$output" == *"JavaScript"* ]]
  run jq -r '.facts.languages.confidence' "$TEST_TMPDIR/repo/.autospec/reports/metadata-discovery.json"
  [ "$output" != "null" ]
  run jq -r '.facts.languages.evidence[0]' "$TEST_TMPDIR/repo/.autospec/reports/metadata-discovery.json"
  [ "$output" != "null" ]
}

@test "metadata discovery reports missing documentation coverage" {
  mkdir -p "$TEST_TMPDIR/repo/src" "$TEST_TMPDIR/repo/tests"
  write_autospec_config "$TEST_TMPDIR/repo"
  printf 'print("hello")\n' > "$TEST_TMPDIR/repo/src/app.py"
  printf 'def test_ok(): pass\n' > "$TEST_TMPDIR/repo/tests/test_app.py"

  run bash "$DISCOVER" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 0 ]
  run jq -r '.status' "$TEST_TMPDIR/repo/.autospec/state/docs-coverage-map.json"
  [ "$output" = "missing" ]
  grep -q 'Documentation coverage: missing' "$TEST_TMPDIR/repo/.autospec/reports/metadata-discovery.md"
}

@test "metadata discovery detects UI and API indicators" {
  mkdir -p "$TEST_TMPDIR/repo/src/pages" "$TEST_TMPDIR/repo/api"
  write_autospec_config "$TEST_TMPDIR/repo"
  printf 'export default function App() { return <main />; }\n' > "$TEST_TMPDIR/repo/src/pages/App.jsx"
  printf 'openapi: 3.0.0\n' > "$TEST_TMPDIR/repo/api/openapi.yml"
  printf '{"scripts":{"test":"vitest run"},"dependencies":{"react":"latest","fastapi":"latest"}}\n' > "$TEST_TMPDIR/repo/package.json"

  run bash "$DISCOVER" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 0 ]
  run jq -r '.indicators.ui.value' "$TEST_TMPDIR/repo/.autospec/reports/metadata-discovery.json"
  [ "$output" = "true" ]
  run jq -r '.indicators.api.value' "$TEST_TMPDIR/repo/.autospec/reports/metadata-discovery.json"
  [ "$output" = "true" ]
}

@test "baseline gap analysis reports present missing unknown and opted out statuses" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_simple_repo "$TEST_TMPDIR/repo"
  write_composition "$TEST_TMPDIR/repo"
  bash "$DISCOVER" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  run bash "$BASELINE_GAP" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/baseline-gap-analysis.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/baseline-gap-analysis.md" ]
  run jq -r '.matrix[] | select(.capability=="documentation") | .status' \
    "$TEST_TMPDIR/repo/.autospec/reports/baseline-gap-analysis.json"
  [ "$output" = "present" ]
  run jq -r '.matrix[] | select(.capability=="ui") | .status' \
    "$TEST_TMPDIR/repo/.autospec/reports/baseline-gap-analysis.json"
  [ "$output" = "missing" ]
  run jq -r '.matrix[] | select(.capability=="manual-review") | .status' \
    "$TEST_TMPDIR/repo/.autospec/reports/baseline-gap-analysis.json"
  [ "$output" = "opted_out" ]
  run jq -r '.matrix[] | select(.capability=="unknown-control") | .status' \
    "$TEST_TMPDIR/repo/.autospec/reports/baseline-gap-analysis.json"
  [ "$output" = "unknown" ]
  grep -q '| web | ui | missing |' "$TEST_TMPDIR/repo/.autospec/reports/baseline-gap-analysis.md"
}

@test "baseline gap analysis reports partial API coverage" {
  mkdir -p "$TEST_TMPDIR/repo/src"
  write_autospec_config "$TEST_TMPDIR/repo"
  printf 'app.get("/health", () => {});\n' > "$TEST_TMPDIR/repo/src/server.js"
  write_composition "$TEST_TMPDIR/repo"
  bash "$DISCOVER" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  run bash "$BASELINE_GAP" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  run jq -r '.matrix[] | select(.capability=="api") | .status' \
    "$TEST_TMPDIR/repo/.autospec/reports/baseline-gap-analysis.json"
  [ "$output" = "partial" ]
}

@test "constitutional gap report suggests next issues without creating issues" {
  mkdir -p "$TEST_TMPDIR/repo" "$TEST_TMPDIR/autospec-constitution"
  write_simple_repo "$TEST_TMPDIR/repo"
  write_constitution "$TEST_TMPDIR/autospec-constitution"
  write_composition "$TEST_TMPDIR/repo"
  bash "$DISCOVER" --repo-root "$TEST_TMPDIR/repo" >/dev/null
  bash "$BASELINE_GAP" --repo-root "$TEST_TMPDIR/repo" >/dev/null || true

  run bash "$CONSTITUTIONAL_GAP" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/constitutional-gap-report.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/constitutional-gap-report.md" ]
  run jq -r '.sections."ui_ux_gaps".status' "$TEST_TMPDIR/repo/.autospec/reports/constitutional-gap-report.json"
  [ "$output" = "gap" ]
  run jq -r '.next_recommended_issues[0].title' "$TEST_TMPDIR/repo/.autospec/reports/constitutional-gap-report.json"
  [[ "$output" == feat:* ]]
  grep -q '## Next Recommended Issues' "$TEST_TMPDIR/repo/.autospec/reports/constitutional-gap-report.md"
}

#!/usr/bin/env bats
# tests/unit/test_constitution_rules.bats — Constitution rule interpretation v1.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  BUILD_TWIN="$REPO_ROOT/scripts/autospec-build-digital-twin.sh"
  EXTRACT="$REPO_ROOT/scripts/autospec-extract-constitution-rules.sh"
  CHECK="$REPO_ROOT/scripts/autospec-check-rules.sh"
  GAP_V1="$REPO_ROOT/scripts/autospec-constitutional-gap-v1.sh"
  AUDIT="$REPO_ROOT/scripts/autospec-constitution-audit.sh"
  VERIFY="$REPO_ROOT/scripts/autospec-verify-worker-pr.sh"
  STATUS="$REPO_ROOT/scripts/autospec-autonomy-status.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-constitution-rules-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_constitution_repo() {
  local root="$1"
  mkdir -p "$root/doctrine" "$root/rules"
  cat > "$root/rules/testing.yml" <<'YAML'
rules:
  - rule_id: testing.playwright.required_for_web
    title: Playwright browser-level testing
    severity: required
    category: testing
    profile: web
    maturity_level: production
    applies_when:
      application.type: web
    check_type: required_tool
    expected:
      tool: playwright
    acceptance_criteria:
      - Playwright is present in dependency metadata.
    evidence_required:
      - technology-registry
    remediation_hint: Add Playwright responsive workflow coverage.
  - rule_id: architecture.no_chart_sprawl
    title: Visualization libraries should be standardized
    severity: recommended
    category: architecture
    profile: web
    maturity_level: prototype
    check_type: forbidden_dependency_sprawl
    expected:
      category: chart_library
    acceptance_criteria:
      - Only one charting library is used or a waiver explains the exception.
    remediation_hint: Consolidate visualization libraries.
YAML
  cat > "$root/doctrine/documentation.md" <<'MD'
# Documentation Doctrine

## In-app documentation center

Web applications should include documentation for users. Required: docs must
exist for the application and should be discoverable.

## Ambiguous Human Judgment

Teams should make excellent decisions with good taste.
MD
}

write_baselines_repo() {
  local root="$1"
  mkdir -p "$root/profiles/web"
  cat > "$root/profiles/web/rules.json" <<'JSON'
{
  "rules": [
    {
      "rule_id": "metadata.digital_twin.required",
      "title": "Digital Twin metadata should be generated",
      "severity": "required",
      "category": "metadata",
      "profile": "web",
      "maturity_level": "production",
      "check_type": "required_metadata",
      "expected": {"file": ".autospec/state/digital-twin.json"},
      "acceptance_criteria": ["Digital Twin state exists."],
      "remediation_hint": "Run scripts/autospec-build-digital-twin.sh."
    }
  ]
}
JSON
}

write_app_repo() {
  local repo="$1"
  mkdir -p "$repo/src/pages" "$repo/src/components" "$repo/docs" "$repo/tests/unit"
  cat > "$repo/.autospec/autospec.yml" <<YAML
constitution:
  source: local
  path: $TEST_TMPDIR/constitution
  version: 0.1.0
baselines:
  source: local
  path: $TEST_TMPDIR/baselines
  profiles:
    - web
application:
  type: web
  maturity_target: production
YAML
  cat > "$repo/package.json" <<'JSON'
{
  "name": "rules-fixture",
  "scripts": {"test": "vitest run"},
  "dependencies": {"react": "^18.0.0", "recharts": "^2.0.0", "chart.js": "^4.0.0"},
  "devDependencies": {"vitest": "^1.0.0"}
}
JSON
  printf '# Rules Fixture\n\n## Create Project\n' > "$repo/README.md"
  printf '# User Docs\n' > "$repo/docs/USER.md"
  printf 'export default function App(){ return null }\n' > "$repo/src/pages/App.tsx"
  printf 'test("works", () => {})\n' > "$repo/tests/unit/app.test.ts"
}

write_rule_waivers() {
  local repo="$1"
  mkdir -p "$repo/.autospec/state"
  cat > "$repo/.autospec/state/rule-waivers.yml" <<'YAML'
waivers:
  - rule_id: architecture.no_chart_sprawl
    status: waived
    reason: "Migration in progress."
    owner: "berlinguyinca"
    expires: "2026-12-31"
    risk: low
  - rule_id: unknown.rule
    status: waived
    owner: "berlinguyinca"
  - rule_id: docs.expired
    status: waived
    reason: "Old exception."
    owner: "berlinguyinca"
    expires: "2020-01-01"
opt_outs:
  - capability: ai_assistant
    status: opted_out
    reason: "No interactive app surface."
    owner: "berlinguyinca"
YAML
}

prepare_repo() {
  mkdir -p "$TEST_TMPDIR/repo/.autospec"
  write_constitution_repo "$TEST_TMPDIR/constitution"
  write_baselines_repo "$TEST_TMPDIR/baselines"
  write_app_repo "$TEST_TMPDIR/repo"
  write_rule_waivers "$TEST_TMPDIR/repo"
  bash "$BUILD_TWIN" --repo-root "$TEST_TMPDIR/repo" >/dev/null
}

@test "rule extraction resolves structured markdown baseline effective rules and waivers" {
  prepare_repo

  run bash "$EXTRACT" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/constitution-rules.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/baseline-rules.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/effective-rules.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/rule-extraction.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/effective-rules.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/rule-waivers.md" ]

  run jq -r '.rules[] | select(.rule_id=="testing.playwright.required_for_web") | .check_type' "$TEST_TMPDIR/repo/.autospec/state/constitution-rules.json"
  [ "$output" = "required_tool" ]
  run jq -r '.rules[] | select(.check_type=="manual_review") | .rule_id' "$TEST_TMPDIR/repo/.autospec/state/constitution-rules.json"
  [[ "$output" == *"manual_review"* ]]
  run jq -r '.rules[] | select(.rule_id=="metadata.digital_twin.required") | .source_type' "$TEST_TMPDIR/repo/.autospec/state/baseline-rules.json"
  [ "$output" = "baseline" ]
  run jq -r '.rules[] | select(.rule_id=="architecture.no_chart_sprawl") | .resolution' "$TEST_TMPDIR/repo/.autospec/state/effective-rules.json"
  [ "$output" = "waived" ]
  run jq -r '.waiver_findings[].code' "$TEST_TMPDIR/repo/.autospec/reports/rule-extraction.json"
  [[ "$output" == *"WAIVER_MISSING_REQUIRED_FIELD"* ]]
  [[ "$output" == *"WAIVER_EXPIRED"* ]]
}

@test "rule check engine evaluates active rules and preserves manual review waived opted-out states" {
  prepare_repo
  bash "$EXTRACT" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  run bash "$CHECK" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 1 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/rule-check-results.md" ]
  run jq -r '.results[] | select(.rule_id=="testing.playwright.required_for_web") | .status' "$TEST_TMPDIR/repo/.autospec/reports/rule-check-results.json"
  [ "$output" = "fail" ]
  run jq -r '.results[] | select(.rule_id=="metadata.digital_twin.required") | .status' "$TEST_TMPDIR/repo/.autospec/reports/rule-check-results.json"
  [ "$output" = "pass" ]
  run jq -r '.results[] | select(.rule_id=="architecture.no_chart_sprawl") | .status' "$TEST_TMPDIR/repo/.autospec/reports/rule-check-results.json"
  [ "$output" = "waived" ]
  run jq -r '.results[] | select(.status=="manual_review") | .confidence' "$TEST_TMPDIR/repo/.autospec/reports/rule-check-results.json"
  [ "$output" != "null" ]
}

@test "constitutional gap v1 maturity and issue plan v2 are generated from rule results" {
  prepare_repo
  bash "$EXTRACT" --repo-root "$TEST_TMPDIR/repo" >/dev/null
  bash "$CHECK" --repo-root "$TEST_TMPDIR/repo" >/dev/null || true

  run bash "$GAP_V1" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 1 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/constitutional-gap-report-v1.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/maturity-score.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/issue-plan-v2.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/backlog/issues-v2/001-testing-playwright-required-for-web.md" ]
  run jq -r '.scorecard.testing.required_fail' "$TEST_TMPDIR/repo/.autospec/reports/constitutional-gap-report-v1.json"
  [ "$output" = "1" ]
  run jq -r '.levels[] | select(.level=="production") | .status' "$TEST_TMPDIR/repo/.autospec/reports/maturity-score.json"
  [ "$output" = "partial" ]
  grep -q 'testing.playwright.required_for_web' "$TEST_TMPDIR/repo/.autospec/backlog/issues-v2/001-testing-playwright-required-for-web.md"
}

@test "unified audit runs all rule stages without GitHub writes" {
  prepare_repo

  run bash "$AUDIT" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 1 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/constitution-audit.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/constitution-audit.json" ]
  run jq -r '.side_effects.github_writes' "$TEST_TMPDIR/repo/.autospec/reports/constitution-audit.json"
  [ "$output" = "false" ]
  grep -q '## Executive Summary' "$TEST_TMPDIR/repo/.autospec/reports/constitution-audit.md"
}

@test "verifier warns on autospec issue missing rule IDs and status reports maturity freshness" {
  prepare_repo
  bash "$AUDIT" --repo-root "$TEST_TMPDIR/repo" >/dev/null || true
  mkdir -p "$TEST_TMPDIR/repo/.autospec/reports" "$TEST_TMPDIR/repo/.autospec/state" "$TEST_TMPDIR/repo/.autospec/backlog/issues"
  cat > "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json" <<'JSON'
{"version":1,"issues":[{"issue_id":"001-no-rule","title":"fix: no rule","suggested_labels":["autospec:managed"],"draft_path":".autospec/backlog/issues/001-no-rule.md","acceptance_criteria":["Validation passes."]}]}
JSON
  printf '# fix: no rule\n\n## Acceptance criteria\n- [ ] Validation passes.\n' > "$TEST_TMPDIR/repo/.autospec/backlog/issues/001-no-rule.md"
  printf '# Packet\n001-no-rule\n' > "$TEST_TMPDIR/repo/.autospec/state/implementation-packet.md"
  printf '{"version":1,"processed_issue_id":"001-no-rule","classification":"docs-only"}\n' > "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json"
  printf '{"version":1,"files_changed":[],"forbidden_path_check":{"passed":true},"patch_budget":{"passed":true}}\n' > "$TEST_TMPDIR/repo/.autospec/reports/worker-diff-review.json"
  printf '{"version":1,"status":"pass"}\n' > "$TEST_TMPDIR/repo/.autospec/reports/worker-validation.json"
  printf '{"version":1,"focused_validation":[],"full_validation":[]}\n' > "$TEST_TMPDIR/repo/.autospec/reports/worker-validation-plan.json"

  run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --dry-run --work-item "$TEST_TMPDIR/repo/.autospec/state"
  [ "$status" -eq 1 ]
  run jq -r '.dimensions[] | select(.dimension=="rule_traceability") | .status' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json"
  [ "$output" = "warn" ]

  run bash "$STATUS" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  grep -q 'Rule Audit' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-status.md"
  run jq -r '.rule_audit.maturity_status' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-status.json"
  [ "$output" = "partial" ]
}

@test "constitution rule batch adds no GitHub Actions cron or scheduler automation" {
  ! git -C "$REPO_ROOT" diff --name-only | grep -Eq '^\\.github/workflows/|cron|launchd|systemd'
}

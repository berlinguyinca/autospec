#!/usr/bin/env bats
# tests/unit/test_spec_implementation_sweep.bats — coverage-driven doctrine implementation sweep.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-doctrine-sweep-XXXXXX)"
  SWEEP="$REPO_ROOT/scripts/autospec-spec-implementation-sweep.sh"
  ARCH="$REPO_ROOT/scripts/autospec-architecture-governance.sh"
  UI="$REPO_ROOT/scripts/autospec-ui-ux-audit.sh"
  PW="$REPO_ROOT/scripts/autospec-playwright-evidence-audit.sh"
  DOCS="$REPO_ROOT/scripts/autospec-doc-artifact-audit.sh"
  REPORTING="$REPO_ROOT/scripts/autospec-reporting-analytics-audit.sh"
  AI="$REPO_ROOT/scripts/autospec-ai-platform-audit.sh"
  NLAI="$REPO_ROOT/scripts/autospec-nlai-audit.sh"
  DIAG="$REPO_ROOT/scripts/autospec-diagnostics-audit.sh"
  DEP="$REPO_ROOT/scripts/autospec-dependency-governance.sh"
  MODERN="$REPO_ROOT/scripts/autospec-modernization-plan.sh"
  SEC="$REPO_ROOT/scripts/autospec-security-privacy-audit.sh"
  DOCTRINE="$REPO_ROOT/scripts/autospec-doctrine-audit.sh"
  COVERAGE="$REPO_ROOT/scripts/autospec-spec-coverage.sh"
  CHECK="$REPO_ROOT/scripts/autospec-check-rules.sh"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_repo() {
  local repo="$1"
  mkdir -p "$repo/.autospec/state" "$repo/.autospec/reports" "$repo/docs/specs" "$repo/src/components" "$repo/tests/e2e" "$repo/docs/tutorials"
  cp -R "$REPO_ROOT/.autospec/templates" "$repo/.autospec/templates"
  cat > "$repo/package.json" <<'JSON'
{"dependencies":{"react":"^18.0.0","recharts":"^2.0.0","chart.js":"^4.0.0"},"devDependencies":{"@playwright/test":"^1.0.0","vitest":"^1.0.0","jest":"^29.0.0"}}
JSON
  echo '{}' > "$repo/package-lock.json"
  cat > "$repo/playwright.config.ts" <<'TS'
export default { projects: [{ name: 'chromium' }] }
TS
  cat > "$repo/tests/e2e/app.spec.ts" <<'TS'
test('captures screenshot', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.screenshot({ path: 'artifacts/home.png' })
})
TS
  echo "# README" > "$repo/README.md"
  echo "# User Guide" > "$repo/docs/user-guide.md"
  echo "# Metrics" > "$repo/docs/metrics.md"
  echo "# Threat Model" > "$repo/docs/threat-model.md"
  echo "export const tokens = { spacing: 8 }" > "$repo/src/components/tokens.ts"
  cat > "$repo/.autospec/state/digital-twin.json" <<'JSON'
{"schema":1,"repo":"fixture","summary":{"application_type":"web","detected_capabilities":3},"confidence":0.7}
JSON
  cat > "$repo/.autospec/state/technology-registry.yml" <<'YAML'
technologies:
  - name: Playwright
    category: testing
  - name: Recharts
    category: chart_library
  - name: Chart.js
    category: chart_library
YAML
  cat > "$repo/.autospec/state/capability-registry.json" <<'JSON'
{"schema":1,"capabilities":[{"id":"nlai-capability-interface","title":"NLAI Capability Interface","type":"ai","evidence":["template exists"],"confidence":0.5}]}
JSON
  cat > "$repo/.autospec/reports/spec-coverage.json" <<'JSON'
{"schema":1,"requirements":[{"id":"ai.token_usage.multi_user_tracking","category":"ai_platform","priority":"high","status":"scaffolded","requirement_type":"target_app_scaffold","risk":"medium"},{"id":"engineering.design_patterns_adrs","category":"engineering","priority":"medium","status":"validated","requirement_type":"validator","risk":"low"},{"id":"docs.drift_detection","category":"docs_tutorial_pdf","priority":"medium","status":"partial","requirement_type":"validator","risk":"low"}]}
JSON
  cp "$repo/.autospec/reports/spec-coverage.json" "$repo/.autospec/state/master-requirements.json"
}

@test "spec implementation sweep classifies coverage requirements safely" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"

  run bash "$SWEEP" --repo-root "$TEST_TMPDIR/repo" --dry-run --priority high,medium
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/spec-implementation-sweep-plan.md" ]
  run jq -r '.classification_counts.can_add_scaffold > 0' "$TEST_TMPDIR/repo/.autospec/reports/spec-implementation-sweep-plan.json"
  [ "$output" = "true" ]
  run jq -r '.side_effects.github_writes' "$TEST_TMPDIR/repo/.autospec/reports/spec-implementation-sweep-result.json"
  [ "$output" = "false" ]
}

@test "architecture governance detects missing ADRs and generates pattern guidance" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"

  run bash "$ARCH" --repo-root "$TEST_TMPDIR/repo" --dry-run --file src/components/form.tsx
  [ "$status" -eq 0 ]
  grep -q "Pattern guidance" "$TEST_TMPDIR/repo/.autospec/reports/architecture-governance.md"
  run jq -r '.checks.adrs.status' "$TEST_TMPDIR/repo/.autospec/reports/architecture-governance.json"
  [ "$output" = "fail" ]
  [ -f "$REPO_ROOT/.autospec/templates/architecture/adr-template.md" ]
}

@test "UI/UX and Playwright audits detect design tokens and viewport/screenshot evidence" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"

  run bash "$UI" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  run jq -r '.checks.design_tokens.status' "$TEST_TMPDIR/repo/.autospec/reports/ui-ux-audit.json"
  [ "$output" = "pass" ]
  grep -q "raw JSON avoidance" "$TEST_TMPDIR/repo/.autospec/reports/ui-ux-audit.md"

  run bash "$PW" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  run jq -r '.checks.playwright_config.status' "$TEST_TMPDIR/repo/.autospec/reports/playwright-evidence-audit.json"
  [ "$output" = "pass" ]
  run jq -r '.checks.viewport_matrix.status' "$TEST_TMPDIR/repo/.autospec/reports/playwright-evidence-audit.json"
  [[ "$output" =~ ^(partial|fail|pass)$ ]]
}

@test "documentation reporting AI NLAI diagnostics dependency and security audits write reports" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"

  for cmd in "$DOCS" "$REPORTING" "$AI" "$NLAI" "$DIAG" "$DEP" "$SEC"; do
    run bash "$cmd" --repo-root "$TEST_TMPDIR/repo" --dry-run
    [ "$status" -eq 0 ]
  done

  [ -f "$TEST_TMPDIR/repo/.autospec/reports/doc-artifact-audit.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/reporting-analytics-audit.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/ai-platform-audit.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/nlai-audit.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/diagnostics-audit.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/dependency-governance.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/security-privacy-audit.md" ]
  grep -q "chart" "$TEST_TMPDIR/repo/.autospec/reports/reporting-analytics-audit.md"
  grep -q "token usage" "$TEST_TMPDIR/repo/.autospec/reports/ai-platform-audit.md"
}

@test "modernization planner creates backlog without changing dependencies" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"
  before="$(sha256sum "$TEST_TMPDIR/repo/package.json" | awk '{print $1}')"

  run bash "$MODERN" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  after="$(sha256sum "$TEST_TMPDIR/repo/package.json" | awk '{print $1}')"
  [ "$before" = "$after" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/modernization-plan.md" ]
  compgen -G "$TEST_TMPDIR/repo/.autospec/backlog/modernization/*.md" >/dev/null
}

@test "unified doctrine audit runs all audits and creates issue drafts" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"

  run bash "$DOCTRINE" --repo-root "$TEST_TMPDIR/repo" --dry-run --all
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/doctrine-audit.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/doctrine-issue-plan.json" ]
  compgen -G "$TEST_TMPDIR/repo/.autospec/backlog/doctrine/*.md" >/dev/null
  run jq -r '.side_effects.github_writes' "$TEST_TMPDIR/repo/.autospec/reports/doctrine-audit.json"
  [ "$output" = "false" ]
}

@test "new doctrine rule check types are supported and spec coverage recognizes audit evidence" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/repo/.autospec/state/effective-rules.json" <<'JSON'
{"schema":1,"rules":[
  {"rule_id":"ai.provider","title":"AI provider","category":"ai","severity":"required","resolution":"active","check_type":"required_ai_provider_abstraction","expected":{}},
  {"rule_id":"docs.pdf","title":"PDF guides","category":"docs","severity":"required","resolution":"active","check_type":"required_pdf_guides","expected":{}},
  {"rule_id":"security.threat","title":"Threat model","category":"security","severity":"required","resolution":"active","check_type":"required_threat_model","expected":{}}
]}
JSON

  run bash "$CHECK" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -ne 127 ]
  run jq -r '.results[] | select(.rule_id=="security.threat") | .status' "$TEST_TMPDIR/repo/.autospec/reports/rule-check-results.json"
  [[ "$output" =~ ^(pass|partial|fail|unknown|manual_review)$ ]]

  bash "$DOCTRINE" --repo-root "$TEST_TMPDIR/repo" --dry-run --all >/dev/null
  run bash "$COVERAGE" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  grep -q "architecture-governance" "$TEST_TMPDIR/repo/.autospec/reports/spec-coverage.md"
}

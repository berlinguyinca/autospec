#!/usr/bin/env bats
# tests/unit/test_mvp_flows.bats — Autospec Constitution MVP operator flows.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  ONBOARD="$REPO_ROOT/scripts/autospec-onboard-existing-repo.sh"
  BOOTSTRAP="$REPO_ROOT/scripts/autospec-bootstrap-new-project.sh"
  AI_SCAFFOLD="$REPO_ROOT/scripts/autospec-generate-ai-nlai-scaffold.sh"
  PRODUCT_SCAFFOLD="$REPO_ROOT/scripts/autospec-generate-product-baseline-scaffold.sh"
  START="$REPO_ROOT/scripts/autospec-start.sh"
  MVP_STATUS="$REPO_ROOT/scripts/autospec-mvp-status.sh"
  WORKER="$REPO_ROOT/scripts/autospec-worker-v1.sh"
  VERIFY="$REPO_ROOT/scripts/autospec-verify-worker-pr.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-mvp-flows-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_existing_repo() {
  local repo="$1"
  mkdir -p "$repo/.autospec/reports" "$repo/.autospec/state" "$repo/src" "$repo/tests" "$repo/docs"
  cat > "$repo/.autospec/autospec.yml" <<'YAML'
application:
  type: web
  maturity_target: production
baselines:
  profiles:
    - web
    - ai-platform
autonomy:
  worker:
    allow_code_changes: true
    code_change_mode: low_risk_only
YAML
  echo "console.log('hello autospec')" > "$repo/src/app.js"
  echo "# Example" > "$repo/README.md"
  echo "test placeholder" > "$repo/tests/app.test.js"
  cat > "$repo/.autospec/state/technology-registry.yml" <<'YAML'
technologies:
  - name: JavaScript
    category: language
    confidence: 0.9
YAML
  cat > "$repo/.autospec/state/capability-registry.json" <<'JSON'
{"capabilities":[{"id":"cli-status","title":"Status command","type":"cli","files":["scripts/autospec-autonomy-status.sh"],"evidence":["script exists"],"confidence":0.8}]}
JSON
  cat > "$repo/.autospec/state/digital-twin.json" <<'JSON'
{"schema":1,"repo":"example","summary":{"application_type":"web","detected_capabilities":1},"confidence":0.7,"warnings":[]}
JSON
  cat > "$repo/.autospec/state/ai-capabilities.json" <<'JSON'
{"facts":[{"name":"ai_assistant","status":"missing","evidence":["ai baseline selected"],"confidence":0.8}]}
JSON
  cat > "$repo/.autospec/state/rule-check-results.json" <<'JSON'
{"results":[
  {"rule_id":"ai.provider_abstraction.required","title":"Provider abstraction","status":"fail","severity":"required","category":"ai","missing_evidence":["provider abstraction"],"evidence":[],"acceptance_criteria":["Provider abstraction spec exists."],"remediation_hint":"Generate AI platform scaffold.","source_file":"rules/ai-platform.yml","source_pack":"application/ai-platform","risk":{"level":"low"}},
  {"rule_id":"docs.in_app_docs.required_for_web_apps","title":"In-app docs","status":"fail","severity":"required","category":"documentation","missing_evidence":["docs center"],"evidence":[],"acceptance_criteria":["Docs center spec exists."],"remediation_hint":"Generate docs center scaffold.","source_file":"rules/documentation.yml","source_pack":"application/web","risk":{"level":"low"}},
  {"rule_id":"testing.playwright.required_for_web","title":"Playwright","status":"fail","severity":"required","category":"testing","missing_evidence":["playwright"],"evidence":[],"acceptance_criteria":["Playwright validation evidence exists."],"remediation_hint":"Add Playwright coverage.","source_file":"rules/testing.yml","source_pack":"application/web","risk":{"level":"low"}}
]}
JSON
  cp "$repo/.autospec/state/rule-check-results.json" "$repo/.autospec/reports/rule-check-results.json"
  cat > "$repo/.autospec/state/quality-gates.json" <<'JSON'
{"gates":[{"id":"testing.playwright.viewport_matrix","source_rule_id":"testing.playwright.required_for_web","status":"fail","required_evidence":["viewport matrix"]}]}
JSON
  cat > "$repo/.autospec/reports/issue-plan-v3.json" <<'JSON'
{"issues":[{"issue_id":"001-playwright","title":"test: add Playwright validation evidence","source_rule_ids":["testing.playwright.required_for_web"],"quality_gate_ids":["testing.playwright.viewport_matrix"],"source_doctrine":"testing","source_baseline_pack":"application/web","source_file":"rules/testing.yml","rule_severity":"required","maturity_level":"production","category":"testing","evidence":[],"missing_evidence":["playwright"],"remediation_hint":"Add Playwright coverage.","acceptance_criteria":["Playwright validation evidence exists."],"validation_expectations":["bash scripts/autospec-check-rules.sh"],"risk":{"level":"low","requires_human_review":false,"requires_architecture_review":false},"draft_path":".autospec/backlog/issues-v3/001-playwright.md"}]}
JSON
  mkdir -p "$repo/.autospec/backlog/issues-v3"
  cat > "$repo/.autospec/backlog/issues-v3/001-playwright.md" <<'MD'
# test: add Playwright validation evidence

## Acceptance criteria
- [ ] Playwright validation evidence exists.
MD
}

@test "existing repository onboarding generates metadata reports and clarification drafts" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_existing_repo "$TEST_TMPDIR/repo"

  run bash "$ONBOARD" --repo-root "$TEST_TMPDIR/repo" --dry-run --profiles web,ai-platform
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/onboarding-plan.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/onboarding-result.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/onboarding.json" ]
  run jq -r '.mode' "$TEST_TMPDIR/repo/.autospec/state/onboarding.json"
  [ "$output" = "existing_repo" ]
  compgen -G "$TEST_TMPDIR/repo/.autospec/backlog/clarifications/*.md" >/dev/null
}

@test "new project bootstrap creates metadata foundation and blueprint, while missing inputs produce questionnaire" {
  mkdir -p "$TEST_TMPDIR/new"

  run bash "$BOOTSTRAP" --repo-root "$TEST_TMPDIR/new" --dry-run --profiles web
  [ "$status" -eq 0 ]
  grep -q "Guided questionnaire" "$TEST_TMPDIR/new/.autospec/reports/bootstrap-plan.md"

  run bash "$BOOTSTRAP" --repo-root "$TEST_TMPDIR/new" --confirm --name example --profiles web,ai-platform --application-type web --maturity-target production --description "Example AI web app"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/new/.autospec/state/product-purpose.md" ]
  [ -f "$TEST_TMPDIR/new/.autospec/state/knowledge-graph.json" ]
  compgen -G "$TEST_TMPDIR/new/docs/specs/*-project-blueprint.md" >/dev/null
  grep -q "Project Blueprint" "$TEST_TMPDIR/new/.autospec/reports/bootstrap-result.md"
}

@test "rule-aware worker writes before-after rule progress and verifier validates it" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_existing_repo "$TEST_TMPDIR/repo"

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/repo" --dry-run --issue-id 001-playwright
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/worker-rule-progress.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/work-items/001-playwright/rule-progress.md" ]
  grep -q "## Rule Progress" "$TEST_TMPDIR/repo/.autospec/reports/worker-pr-body.md"

  run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --dry-run --work-item "$TEST_TMPDIR/repo/.autospec/state/work-items/001-playwright"
  [ "$status" -eq 0 ]
  grep -q "Rule Progress Verification" "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.md"
  run jq -r '.rule_progress_verification.status' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json"
  [ "$output" != "missing" ]
}

@test "AI and NLAI scaffold generator writes specs and v3 issue drafts from Digital Twin evidence" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_existing_repo "$TEST_TMPDIR/repo"

  run bash "$AI_SCAFFOLD" --repo-root "$TEST_TMPDIR/repo" --confirm --capability ai-assistant
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/ai-nlai-scaffold-result.md" ]
  compgen -G "$TEST_TMPDIR/repo/docs/specs/*-ai-platform-scaffold.md" >/dev/null
  compgen -G "$TEST_TMPDIR/repo/docs/specs/*-nlai-capability-interface.md" >/dev/null
  compgen -G "$TEST_TMPDIR/repo/.autospec/backlog/issues-v3/*ai*.md" >/dev/null
  grep -R "Token usage" "$TEST_TMPDIR/repo/.autospec/backlog/issues-v3" >/dev/null
}

@test "product baseline scaffold generator writes docs/settings/reporting/diagnostics specs and issues" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_existing_repo "$TEST_TMPDIR/repo"

  run bash "$PRODUCT_SCAFFOLD" --repo-root "$TEST_TMPDIR/repo" --confirm --capability docs-center
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/product-baseline-scaffold-result.md" ]
  compgen -G "$TEST_TMPDIR/repo/docs/specs/*-product-baseline-scaffold.md" >/dev/null
  grep -R "In-app documentation" "$TEST_TMPDIR/repo/.autospec/backlog/issues-v3" >/dev/null
  grep -R "Diagnostics" "$TEST_TMPDIR/repo/.autospec/backlog/issues-v3" >/dev/null
}

@test "autospec-start recommends bootstrap, onboarding, audit, backlog, stuck, or review paths" {
  mkdir -p "$TEST_TMPDIR/new" "$TEST_TMPDIR/existing"

  run bash "$START" --repo-root "$TEST_TMPDIR/new" --dry-run
  [ "$status" -eq 0 ]
  grep -q "autospec-bootstrap-new-project.sh" "$TEST_TMPDIR/new/.autospec/reports/start-plan.md"

  write_existing_repo "$TEST_TMPDIR/existing"
  run bash "$START" --repo-root "$TEST_TMPDIR/existing" --dry-run
  [ "$status" -eq 0 ]
  grep -q "autospec-onboard-existing-repo.sh\\|autospec-audit-to-backlog.sh\\|autospec-supervisor-cycle.sh" "$TEST_TMPDIR/existing/.autospec/reports/start-plan.md"
}

@test "MVP status, command index, walkthrough, and known limitations are generated/readable" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_existing_repo "$TEST_TMPDIR/repo"

  run bash "$MVP_STATUS" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/mvp-status.md" ]
  grep -q "Autospec Constitution MVP Status" "$TEST_TMPDIR/repo/.autospec/reports/mvp-status.md"
  [ -f "$REPO_ROOT/docs/runbooks/COMMANDS.md" ]
  [ -f "$REPO_ROOT/docs/runbooks/MVP_WALKTHROUGH.md" ]
  [ -f "$REPO_ROOT/docs/KNOWN_LIMITATIONS.md" ]
  grep -q "autospec-start.sh" "$REPO_ROOT/docs/runbooks/COMMANDS.md"
}

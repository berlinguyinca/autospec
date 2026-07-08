#!/usr/bin/env bats
# tests/unit/test_spec_coverage_closure.bats — full vision spec coverage closure.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  COVERAGE="$REPO_ROOT/scripts/autospec-spec-coverage.sh"
  CHECK="$REPO_ROOT/scripts/autospec-check-rules.sh"
  MVP_STATUS="$REPO_ROOT/scripts/autospec-mvp-status.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-spec-coverage-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_repo() {
  local repo="$1"
  mkdir -p "$repo/.autospec/state" "$repo/.autospec/reports" "$repo/.autospec/backlog/issues-v3" "$repo/docs/specs" "$repo/src/pages" "$repo/src/components" "$repo/tests/e2e" "$repo/scripts"
  cat > "$repo/.autospec/autospec.yml" <<'YAML'
application:
  type: web
  maturity_target: production
baselines:
  profiles:
    - web
    - ai-platform
YAML
  cp -R "$REPO_ROOT/.autospec/templates" "$repo/.autospec/templates"
  cp "$REPO_ROOT/scripts/autospec-build-digital-twin.sh" "$repo/scripts/autospec-build-digital-twin.sh"
  cp "$REPO_ROOT/scripts/autospec-digital-twin.py" "$repo/scripts/autospec-digital-twin.py"
  cp "$REPO_ROOT/scripts/autospec-metadata-drift.sh" "$repo/scripts/autospec-metadata-drift.sh"
  cp "$REPO_ROOT/scripts/autospec-promote-pr.sh" "$repo/scripts/autospec-promote-pr.sh"
  cat > "$repo/.autospec/state/digital-twin.json" <<'JSON'
{"schema":1,"repo":"fixture","summary":{"application_type":"web","detected_capabilities":3},"confidence":0.7,"warnings":[]}
JSON
  cat > "$repo/.autospec/state/capability-registry.json" <<'JSON'
{"schema":1,"capabilities":[{"id":"autonomy-supervisor","title":"Supervisor","type":"workflow","files":["scripts/autospec-supervisor-cycle.sh"],"evidence":["script exists"],"confidence":0.9},{"id":"digital-twin","title":"Digital Twin","type":"metadata","files":["scripts/autospec-build-digital-twin.sh"],"evidence":["script exists"],"confidence":0.9}]}
JSON
  cat > "$repo/.autospec/state/effective-rules.json" <<'JSON'
{"schema":1,"rules":[{"rule_id":"testing.playwright.viewport_matrix","title":"Viewport matrix","category":"testing","severity":"required","resolution":"active","check_type":"required_playwright_viewport_matrix","expected":{}},{"rule_id":"ai.token_usage.tracking","title":"Token usage","category":"ai","severity":"required","resolution":"active","check_type":"required_token_usage_tracking","expected":{}},{"rule_id":"nlai.pretty_rendering.required","title":"Pretty rendering","category":"ai","severity":"recommended","resolution":"active","check_type":"required_pretty_rendering","expected":{}}]}
JSON
  cat > "$repo/.autospec/state/rule-check-results.json" <<'JSON'
{"schema":1,"results":[{"rule_id":"testing.playwright.viewport_matrix","status":"fail","severity":"required","category":"testing","missing_evidence":["viewport matrix"]},{"rule_id":"ai.token_usage.tracking","status":"fail","severity":"required","category":"ai","missing_evidence":["token usage"]}]}
JSON
  cp "$repo/.autospec/state/rule-check-results.json" "$repo/.autospec/reports/rule-check-results.json"
  cat > "$repo/.autospec/reports/constitution-audit.json" <<'JSON'
{"schema":1,"status":"fail","required_failures":["testing.playwright.viewport_matrix","ai.token_usage.tracking"]}
JSON
  cat > "$repo/.autospec/state/baseline-composition.json" <<'JSON'
{"schema":1,"included_packs":["application/web","application/ai-platform"],"effective_capabilities":["in-app documentation center","token usage tracking"]}
JSON
  cat > "$repo/docs/specs/vision.md" <<'MD'
# Vision

Autospec should cover autonomous development, Constitution, Baselines, Digital Twin, app baselines, AI/RAG/NLAI, diagnostics, testing, Playwright, tutorials, PDFs, reporting, visualization standards, dependency governance, modernization, metadata, and onboarding.
MD
}

@test "spec coverage command generates master inventory report and backlog drafts" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"

  run bash "$COVERAGE" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/master-requirements.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/spec-coverage.md" ]
  [ -f "$TEST_TMPDIR/repo/docs/specs/AUTOSPEC_CONSTITUTION_MASTER_SPEC_COVERAGE.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/spec-coverage-backlog.json" ]
  compgen -G "$TEST_TMPDIR/repo/.autospec/backlog/spec-coverage/*.md" >/dev/null
  run jq -r '.summary.categories.autonomous_development.total > 0' "$TEST_TMPDIR/repo/.autospec/reports/spec-coverage.json"
  [ "$output" = "true" ]
  run jq -r '.requirements[] | select(.id=="ai.token_usage.multi_user_tracking") | .status' "$TEST_TMPDIR/repo/.autospec/state/master-requirements.json"
  [[ "$output" =~ ^(scaffolded|validated|partial|missing)$ ]]
}

@test "coverage matrix includes original vision categories and honest statuses" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"
  bash "$COVERAGE" --repo-root "$TEST_TMPDIR/repo" --dry-run >/dev/null

  for category in autonomous_development policy digital_twin engineering testing ui_ux docs_tutorial_pdf reporting_analytics_visualization ai_platform nlai diagnostics product_baseline; do
    run jq -e --arg c "$category" '.summary.categories[$c].total > 0' "$TEST_TMPDIR/repo/.autospec/reports/spec-coverage.json"
    [ "$status" -eq 0 ]
  done
  run jq -r '[.requirements[].status] | unique | join(",")' "$TEST_TMPDIR/repo/.autospec/state/master-requirements.json"
  [[ "$output" == *"implemented"* ]]
  [[ "$output" == *"scaffolded"* ]]
  [[ "$output" == *"deferred"* ]]
}

@test "release candidate closure rows are implemented with evidence" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"
  bash "$COVERAGE" --repo-root "$TEST_TMPDIR/repo" --dry-run >/dev/null

  for requirement_id in digital_twin.knowledge_graph digital_twin.surfaces docs.drift_detection autonomy.no_self_approval; do
    run jq -r --arg id "$requirement_id" '.requirements[] | select(.id==$id) | .status' "$TEST_TMPDIR/repo/.autospec/reports/spec-coverage.json"
    [ "$status" -eq 0 ]
    [ "$output" = "implemented" ]

    run jq -r --arg id "$requirement_id" '.requirements[] | select(.id==$id) | (.evidence | length > 0)' "$TEST_TMPDIR/repo/.autospec/reports/spec-coverage.json"
    [ "$output" = "true" ]
  done
}

@test "new rule check types are evaluated heuristically without crashing" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/repo/.autospec/state/effective-rules.json" <<'JSON'
{"schema":1,"rules":[
  {"rule_id":"testing.viewport","title":"Viewport matrix","category":"testing","severity":"required","resolution":"active","check_type":"required_playwright_viewport_matrix","expected":{}},
  {"rule_id":"ai.provider","title":"Provider abstraction","category":"ai","severity":"required","resolution":"active","check_type":"required_provider_abstraction","expected":{}},
  {"rule_id":"nlai.rendering","title":"Pretty rendering","category":"ai","severity":"required","resolution":"active","check_type":"required_pretty_rendering","expected":{}},
  {"rule_id":"ops.incident","title":"Incident report","category":"operations","severity":"recommended","resolution":"active","check_type":"required_incident_report_template","expected":{}}
]}
JSON

  run bash "$CHECK" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -ne 127 ]
  run jq -r '.results[] | select(.rule_id=="ai.provider") | .status' "$TEST_TMPDIR/repo/.autospec/reports/rule-check-results.json"
  [[ "$output" =~ ^(pass|partial|fail|unknown|manual_review)$ ]]
  run jq -r '.results[] | select(.rule_id=="nlai.rendering") | .suggested_issue_title' "$TEST_TMPDIR/repo/.autospec/reports/rule-check-results.json"
  [[ "$output" == feat:* ]]
}

@test "scaffold templates contain actionable implementation sections" {
  required_sections=("Purpose" "App-type applicability" "Architecture recommendation" "UI expectations" "Settings/config expectations" "Tests required" "Playwright expectations" "Docs/tutorial expectations" "Security/privacy notes" "Acceptance criteria" "Validation commands" "Metadata files expected to change" "Worker eligibility/risk notes")
  for template in "$REPO_ROOT"/.autospec/templates/ai-platform/*.md "$REPO_ROOT"/.autospec/templates/product-baseline/*.md; do
    for section in "${required_sections[@]}"; do
      grep -q "## $section" "$template"
    done
  done
}

@test "MVP status integrates spec coverage and known limitations are categorized" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo "$TEST_TMPDIR/repo"
  bash "$COVERAGE" --repo-root "$TEST_TMPDIR/repo" --dry-run >/dev/null

  run bash "$MVP_STATUS" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  run jq -r '.spec_coverage.critical_missing_requirements' "$TEST_TMPDIR/repo/.autospec/reports/mvp-status.json"
  [ "$output" != "null" ]
  grep -q "Spec coverage" "$TEST_TMPDIR/repo/.autospec/reports/mvp-status.md"

  for section in "Implemented in engine" "Implemented as target-repo scaffolds" "Validated by policy/rules only" "Deferred beyond MVP" "Not supported by design" "Requires human guidance"; do
    grep -q "## $section" "$REPO_ROOT/docs/KNOWN_LIMITATIONS.md"
  done
  [ -f "$REPO_ROOT/docs/specs/AUTOSPEC_CONSTITUTION_MASTER_SPEC.md" ]
  grep -q "## Beyond-MVP scope" "$REPO_ROOT/docs/specs/AUTOSPEC_CONSTITUTION_MASTER_SPEC.md"
}

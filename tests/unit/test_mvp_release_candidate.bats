#!/usr/bin/env bats
# tests/unit/test_mvp_release_candidate.bats — release-candidate hardening flows.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-rc-XXXXXX)"
  PREFLIGHT="$REPO_ROOT/scripts/autospec-preflight.sh"
  SMOKE="$REPO_ROOT/scripts/autospec-mvp-smoke.sh"
  COMMAND_AUDIT="$REPO_ROOT/scripts/autospec-command-audit.sh"
  REPORT_INDEX="$REPO_ROOT/scripts/autospec-report-index.sh"
  STATE_VALIDATE="$REPO_ROOT/scripts/autospec-validate-state.sh"
  SENSITIVE="$REPO_ROOT/scripts/autospec-sensitive-output-audit.sh"
  RECOVERY="$REPO_ROOT/scripts/autospec-recovery-status.sh"
  CLEAN="$REPO_ROOT/scripts/autospec-clean-generated-reports.sh"
  MVP_STATUS="$REPO_ROOT/scripts/autospec-mvp-status.sh"
  ONBOARD="$REPO_ROOT/scripts/autospec-onboard-existing-repo.sh"
  BOOTSTRAP="$REPO_ROOT/scripts/autospec-bootstrap-new-project.sh"
  AI_SCAFFOLD="$REPO_ROOT/scripts/autospec-generate-ai-nlai-scaffold.sh"
  PRODUCT_SCAFFOLD="$REPO_ROOT/scripts/autospec-generate-product-baseline-scaffold.sh"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_fixture_repo() {
  local repo="$1"
  mkdir -p "$repo/.autospec/state" "$repo/.autospec/reports" "$repo/src" "$repo/tests"
  cat > "$repo/.autospec/autospec.yml" <<'YAML'
constitution:
  source: local
  path: ../autospec-constitution
  version: 0.1.0
baselines:
  source: local
  path: ../autospec-baselines
  profiles:
    - web
application:
  type: web
  maturity_target: production
YAML
  echo "console.log('fixture')" > "$repo/src/app.js"
  echo "test fixture" > "$repo/tests/app.test.js"
  cat > "$repo/.autospec/state/digital-twin.json" <<'JSON'
{"schema":1,"repo":"fixture","summary":{"application_type":"web"},"confidence":0.7,"warnings":[]}
JSON
  cat > "$repo/.autospec/state/rule-check-results.json" <<'JSON'
{"schema":1,"results":[{"rule_id":"docs.readme.required","title":"README","status":"fail","severity":"required","category":"documentation","evidence":[],"missing_evidence":["README"],"confidence":0.8}]}
JSON
  cp "$repo/.autospec/state/rule-check-results.json" "$repo/.autospec/reports/rule-check-results.json"
  cat > "$repo/.autospec/reports/issue-plan-v3.json" <<'JSON'
{"schema":1,"issues":[{"issue_id":"001-readme","title":"docs: add README","source_rule_ids":["docs.readme.required"],"rule_severity":"required","category":"documentation","maturity_level":"production","risk":{"level":"low"},"draft_path":".autospec/backlog/issues-v3/001-readme.md"}]}
JSON
  cat > "$repo/.autospec/reports/maturity-score.json" <<'JSON'
{"schema":1,"levels":[{"level":"production","status":"partial","score":0.5,"blocking_gaps":["docs.readme.required"]}]}
JSON
}

@test "preflight reports local readiness without requiring GitHub" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_fixture_repo "$TEST_TMPDIR/repo"

  run bash "$PREFLIGHT" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/preflight.md" ]
  grep -q "GitHub CLI" "$TEST_TMPDIR/repo/.autospec/reports/preflight.md"
  run jq -r '.verdict' "$TEST_TMPDIR/repo/.autospec/reports/preflight.json"
  [[ "$output" =~ ^(pass|pass_with_warnings|needs_fixes)$ ]]
}

@test "command audit and report index produce readable command/report inventories" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_fixture_repo "$TEST_TMPDIR/repo"

  run bash "$COMMAND_AUDIT" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  grep -q "autospec-start.sh" "$TEST_TMPDIR/repo/.autospec/reports/command-audit.md"
  run jq -r '.summary.commands_total > 0' "$TEST_TMPDIR/repo/.autospec/reports/command-audit.json"
  [ "$output" = "true" ]

  run bash "$REPORT_INDEX" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/REPORT_INDEX.md" ]
  grep -q "Latest Reports" "$TEST_TMPDIR/repo/.autospec/reports/REPORT_INDEX.md"
}

@test "state validation and sensitive-output audit find safe and unsafe artifacts" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_fixture_repo "$TEST_TMPDIR/repo"

  run bash "$STATE_VALIDATE" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  grep -q "State Validation" "$TEST_TMPDIR/repo/.autospec/reports/state-validation.md"

  fake_token="$(printf 'gh%s_%s%s' "p" "12345678901234567890" "1234567890123456")"
  printf 'token=%s\n' "$fake_token" > "$TEST_TMPDIR/repo/.autospec/reports/leaky.md"
  run bash "$SENSITIVE" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -ne 0 ]
  grep -q "REDACTED" "$TEST_TMPDIR/repo/.autospec/reports/sensitive-output-audit.md"
}

@test "recovery status and cleanup are dry-run safe and bounded to reports" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_fixture_repo "$TEST_TMPDIR/repo"
  echo "{}" > "$TEST_TMPDIR/repo/.autospec/reports/temp-report.json"

  run bash "$RECOVERY" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  grep -q "Recovery Status" "$TEST_TMPDIR/repo/.autospec/reports/recovery-status.md"

  run bash "$CLEAN" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  grep -q "temp-report.json" "$TEST_TMPDIR/repo/.autospec/reports/clean-generated-reports.md"
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/temp-report.json" ]
}

@test "local validation foundation smoke dry-run runs local release checks and mvp-status integrates signals" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_fixture_repo "$TEST_TMPDIR/repo"

  run bash "$SMOKE" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/mvp-smoke.md" ]
  grep -q "Autospec MVP Smoke Report" "$TEST_TMPDIR/repo/.autospec/reports/mvp-smoke.md"

  run bash "$MVP_STATUS" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  run jq -r '.readiness' "$TEST_TMPDIR/repo/.autospec/reports/mvp-status.json"
  [[ "$output" =~ ^MVP_(READY|READY_WITH_WARNINGS|NOT_READY|BLOCKED)$ ]]
  grep -q "MVP_READY" "$TEST_TMPDIR/repo/.autospec/reports/mvp-status.md"
}

@test "golden dry-run flows for existing repo and new project complete without network" {
  mkdir -p "$TEST_TMPDIR/existing" "$TEST_TMPDIR/new"
  write_fixture_repo "$TEST_TMPDIR/existing"

  run bash "$PREFLIGHT" --repo-root "$TEST_TMPDIR/existing" --dry-run
  [ "$status" -eq 0 ]
  run bash "$ONBOARD" --repo-root "$TEST_TMPDIR/existing" --dry-run --profiles web
  [ "$status" -eq 0 ]
  run bash "$SMOKE" --repo-root "$TEST_TMPDIR/existing" --dry-run
  [ "$status" -eq 0 ]

  run bash "$BOOTSTRAP" --repo-root "$TEST_TMPDIR/new" --dry-run --name example --profiles web,ai-platform --application-type web
  [ "$status" -eq 0 ]
  run bash "$AI_SCAFFOLD" --repo-root "$TEST_TMPDIR/new" --dry-run
  [ "$status" -eq 0 ]
  run bash "$PRODUCT_SCAFFOLD" --repo-root "$TEST_TMPDIR/new" --dry-run
  [ "$status" -eq 0 ]
  run bash "$MVP_STATUS" --repo-root "$TEST_TMPDIR/new"
  [ "$status" -eq 0 ]
}

@test "release docs and dogfood examples exist and state local-only safety" {
  [ -f "$REPO_ROOT/docs/runbooks/DOGFOODING.md" ]
  [ -f "$REPO_ROOT/.autospec/examples/dogfood-autospec.yml" ]
  [ -f "$REPO_ROOT/docs/RELEASE_READINESS.md" ]
  [ -f "$REPO_ROOT/docs/MIGRATION.md" ]
  grep -q "Autospec Local Validation Foundation" "$REPO_ROOT/docs/runbooks/MVP_WALKTHROUGH.md"
  grep -q "No GitHub Actions" "$REPO_ROOT/docs/RELEASE_READINESS.md"
  grep -q "issue-plan-v1/v2/v3" "$REPO_ROOT/docs/MIGRATION.md"
}

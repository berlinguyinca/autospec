#!/usr/bin/env bats
# tests/unit/test_structured_backlog_pipeline.bats — issue-plan-v3 publishing and autonomy consumption.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  PUBLISH="$REPO_ROOT/scripts/autospec-publish-issues.sh"
  SYNC="$REPO_ROOT/scripts/autospec-sync-published-issues.sh"
  SUPERVISOR="$REPO_ROOT/scripts/autospec-supervisor-cycle.sh"
  WORKER="$REPO_ROOT/scripts/autospec-worker-v1.sh"
  VERIFY="$REPO_ROOT/scripts/autospec-verify-worker-pr.sh"
  PROMOTE="$REPO_ROOT/scripts/autospec-promote-pr.sh"
  STATUS="$REPO_ROOT/scripts/autospec-autonomy-status.sh"
  AUDIT_TO_BACKLOG="$REPO_ROOT/scripts/autospec-audit-to-backlog.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-structured-backlog-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_gh_stub() {
  mkdir -p "$TEST_TMPDIR/bin"
  cat > "$TEST_TMPDIR/bin/gh" <<'SH'
#!/usr/bin/env bash
echo "$*" >> "${GH_STUB_LOG:?}"
if [ "$1" = "--repo" ]; then shift 2; fi
case "$1 $2" in
  "issue create")
    echo "https://github.com/example/repo/issues/123"
    ;;
  "issue view")
    cat <<'JSON'
{"number":123,"url":"https://github.com/example/repo/issues/123","title":"feat: add Playwright coverage","state":"OPEN","labels":[{"name":"autospec:managed"},{"name":"autospec:discovered"}],"body":"<!-- autospec-plan-version: v3 -->\n<!-- autospec-local-issue-id: 001-testing-playwright -->\n<!-- autospec-rule-id: testing.playwright.required_for_web -->\n<!-- autospec-rule-result-hash: old -->\n<!-- autospec-body-hash: old -->"}
JSON
    ;;
  "issue list")
    echo "[]"
    ;;
  "issue edit"|"issue reopen"|"pr comment")
    echo ""
    ;;
  *)
    echo "{}"
    ;;
esac
SH
  chmod +x "$TEST_TMPDIR/bin/gh"
  export GH_STUB_LOG="$TEST_TMPDIR/gh.log"
  export PATH="$TEST_TMPDIR/bin:$PATH"
}

write_repo_base() {
  local repo="$1"
  mkdir -p "$repo/.autospec/reports" "$repo/.autospec/state" "$repo/.autospec/backlog/issues-v3"
  cat > "$repo/.autospec/autospec.yml" <<'YAML'
github:
  issue_publishing:
    enabled: true
    default_mode: dry_run
    require_confirm: true
    apply_labels: true
autonomy:
  worker:
    allow_code_changes: true
    code_change_mode: low_risk_only
YAML
  cat > "$repo/.autospec/state/control-labels.yml" <<'YAML'
labels:
  autospec:managed:
    color: "0e8a16"
  autospec:discovered:
    color: "1d76db"
  autospec:testing:
    color: "c2e0c6"
YAML
}

write_v3_plan() {
  local repo="$1"
  cat > "$repo/.autospec/reports/issue-plan-v3.json" <<'JSON'
{
  "schema": 1,
  "generated_at": "1970-01-01T00:00:00Z",
  "issues": [
    {
      "issue_id": "001-testing-playwright",
      "title": "test: add Playwright coverage",
      "source_rule_ids": ["testing.playwright.required_for_web"],
      "quality_gates": ["Viewport matrix exists"],
      "source_doctrine": "testing",
      "source_baseline_pack": "application/web",
      "source_file": "rules/testing.yml",
      "rule_severity": "required",
      "maturity_level": "production",
      "category": "testing",
      "evidence": [],
      "missing_evidence": ["playwright"],
      "remediation_hint": "Add Playwright viewport coverage.",
      "suggested_labels": ["autospec:testing"],
      "acceptance_criteria": ["Playwright is configured."],
      "validation_expectations": ["bash scripts/autospec-constitution-audit.sh"],
      "metadata_expectations": ["refresh rule-check-results"],
      "risk": {"level": "low", "requires_human_review": false, "requires_architecture_review": false},
      "draft_path": ".autospec/backlog/issues-v3/001-testing-playwright.md"
    },
    {
      "issue_id": "002-security-model",
      "title": "docs: define security model",
      "source_rule_ids": ["security.threat_model.required_for_production"],
      "quality_gates": ["Threat model reviewed"],
      "source_doctrine": "security",
      "source_baseline_pack": "",
      "source_file": "rules/security-privacy.yml",
      "rule_severity": "required",
      "maturity_level": "production",
      "category": "security",
      "evidence": [],
      "missing_evidence": ["threat model"],
      "remediation_hint": "Add threat model.",
      "suggested_labels": ["autospec:architecture"],
      "acceptance_criteria": ["Threat model exists."],
      "validation_expectations": ["bash scripts/autospec-constitution-audit.sh"],
      "metadata_expectations": ["refresh rule-check-results"],
      "risk": {"level": "high", "requires_human_review": true, "requires_architecture_review": true},
      "draft_path": ".autospec/backlog/issues-v3/002-security-model.md"
    }
  ]
}
JSON
  cat > "$repo/.autospec/backlog/issues-v3/001-testing-playwright.md" <<'MD'
# test: add Playwright coverage

## Source
- Rule ID: `testing.playwright.required_for_web`

## Acceptance Criteria
- [ ] Playwright is configured.
MD
  cat > "$repo/.autospec/backlog/issues-v3/002-security-model.md" <<'MD'
# docs: define security model

## Source
- Rule ID: `security.threat_model.required_for_production`
MD
  cat > "$repo/.autospec/state/rule-check-results.json" <<'JSON'
{
  "results": [
    {"rule_id":"testing.playwright.required_for_web","status":"fail","severity":"required","category":"testing","source_file":"rules/testing.yml","source_pack":"application/web","evidence":[],"missing_evidence":["playwright"],"acceptance_criteria":["Playwright is configured."],"quality_gates":["Viewport matrix exists"],"risk":{"level":"low"}},
    {"rule_id":"security.threat_model.required_for_production","status":"fail","severity":"required","category":"security","source_file":"rules/security-privacy.yml","source_pack":"","evidence":[],"missing_evidence":["threat model"],"acceptance_criteria":["Threat model exists."],"quality_gates":["Threat model reviewed"],"risk":{"level":"high","requires_human_review":true,"requires_architecture_review":true}}
  ]
}
JSON
  cp "$repo/.autospec/state/rule-check-results.json" "$repo/.autospec/reports/rule-check-results.json"
  cat > "$repo/.autospec/state/effective-rules.json" <<'JSON'
{"rules":[{"rule_id":"testing.playwright.required_for_web","resolution":"active"},{"rule_id":"security.threat_model.required_for_production","resolution":"active"}]}
JSON
  cat > "$repo/.autospec/state/quality-gates.json" <<'JSON'
{"gates":[{"id":"testing.playwright.required_for_web.gate_1","source_rule_id":"testing.playwright.required_for_web","status":"fail"}]}
JSON
  cat > "$repo/.autospec/state/maturity-score.json" <<'JSON'
{"levels":[{"level":"production","status":"partial","blocking_gaps":["testing.playwright.required_for_web","security.threat_model.required_for_production"]}]}
JSON
  cp "$repo/.autospec/state/maturity-score.json" "$repo/.autospec/reports/maturity-score.json"
}

@test "v3 publishing dry-run prefers newest plan and writes v3 reports with markers" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo_base "$TEST_TMPDIR/repo"
  write_v3_plan "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json" <<'JSON'
{"issues":[{"issue_id":"001-old","title":"old","draft_path":".autospec/backlog/issues/001-old.md"}]}
JSON

  run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-plan-v3.json" ]
  run jq -r '.plan_version' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-plan-v3.json"
  [ "$output" = "v3" ]
  grep -q 'autospec-plan-version: v3' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-plan-v3.md"
  grep -q 'autospec-rule-id: testing.playwright.required_for_web' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-plan-v3.md"
  [ ! -f "$TEST_TMPDIR/repo/.autospec/state/published-issues.json" ]
}

@test "v3 confirm publishes through mocked GitHub and records structured ledger fields" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo_base "$TEST_TMPDIR/repo"
  write_v3_plan "$TEST_TMPDIR/repo"
  write_gh_stub

  run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --confirm --plan v3 --repo example/repo
  [ "$status" -eq 0 ]
  [ -s "$GH_STUB_LOG" ]
  run jq -r '.issues[0].plan_version' "$TEST_TMPDIR/repo/.autospec/state/published-issues.json"
  [ "$output" = "v3" ]
  run jq -r '.issues[0].rule_ids[0]' "$TEST_TMPDIR/repo/.autospec/state/published-issues.json"
  [ "$output" = "testing.playwright.required_for_web" ]
  run jq -r '.summary.github_issues_created' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-result-v3.json"
  [ "$output" = "true" ]
}

@test "v3 sync reports stale disappeared waived closed and duplicates" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo_base "$TEST_TMPDIR/repo"
  write_v3_plan "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/repo/.autospec/state/published-issues.json" <<'JSON'
{"schema":1,"repo":"example/repo","issues":[
  {"local_issue_id":"001-testing-playwright","plan_version":"v3","rule_ids":["testing.playwright.required_for_web"],"github_issue_number":123,"state":"open","source_gap_hash":"old","body_hash":"old"},
  {"local_issue_id":"001-old-playwright","plan_version":"v2","rule_ids":["testing.playwright.required_for_web"],"github_issue_number":99,"state":"open"},
  {"local_issue_id":"003-missing-rule","plan_version":"v3","rule_ids":["missing.rule"],"github_issue_number":124,"state":"closed"},
  {"local_issue_id":"004-waived","plan_version":"v3","rule_ids":["waived.rule"],"github_issue_number":125,"state":"open"}
]}
JSON
  cat > "$TEST_TMPDIR/repo/.autospec/state/rule-check-results.json" <<'JSON'
{"results":[{"rule_id":"testing.playwright.required_for_web","status":"pass","severity":"required"},{"rule_id":"waived.rule","status":"waived","severity":"required"}]}
JSON
  write_gh_stub

  run bash "$SYNC" --repo-root "$TEST_TMPDIR/repo" --repo example/repo
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-issue-sync-v3.md" ]
  run jq -r '.summary.stale_v3_issues' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-sync-v3.json"
  [ "$output" = "1" ]
  run jq -r '.summary.duplicate_issues' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-sync-v3.json"
  [ "$output" = "1" ]
  grep -q 'missing.rule' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-sync-v3.md"
}

@test "supervisor dry-run selects v3 issue first and includes structured context" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo_base "$TEST_TMPDIR/repo"
  write_v3_plan "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/repo/.autospec/state/published-issues.json" <<'JSON'
{"schema":1,"issues":[
  {"local_issue_id":"old","plan_version":"v1","github_issue_number":1,"state":"open"},
  {"local_issue_id":"001-testing-playwright","plan_version":"v3","rule_ids":["testing.playwright.required_for_web"],"quality_gate_ids":["testing.playwright.required_for_web.gate_1"],"github_issue_number":123,"state":"open","category":"testing","severity":"required","maturity_level":"production","source_policy_files":["rules/testing.yml"]}
]}
JSON

  run bash "$SUPERVISOR" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  run jq -r '.selected_issue_context.plan_version' "$TEST_TMPDIR/repo/.autospec/reports/supervisor-cycle-plan.json"
  [ "$output" = "v3" ]
  grep -q 'Source Rule' "$TEST_TMPDIR/repo/.autospec/reports/supervisor-cycle-plan.md"
  grep -q 'testing.playwright.required_for_web' "$TEST_TMPDIR/repo/.autospec/reports/supervisor-cycle-plan.md"
}

@test "worker packet includes structured policy context and high-risk issue is guidance" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo_base "$TEST_TMPDIR/repo"
  write_v3_plan "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/repo/.autospec/state/published-issues.json" <<'JSON'
{"schema":1,"issues":[{"local_issue_id":"002-security-model","plan_version":"v3","rule_ids":["security.threat_model.required_for_production"],"github_issue_number":124,"state":"open","risk":{"level":"high","requires_human_review":true,"requires_architecture_review":true},"category":"security","severity":"required","maturity_level":"production"}]}
JSON

  run bash "$WORKER" --repo-root "$TEST_TMPDIR/repo" --dry-run --issue 124
  [ "$status" -eq 1 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/work-items/124/implementation-packet.md" ]
  grep -q '## Structured Policy Context' "$TEST_TMPDIR/repo/.autospec/state/work-items/124/implementation-packet.md"
  grep -q 'Required capability level' "$TEST_TMPDIR/repo/.autospec/state/work-items/124/stuck-handoff.md"
  run jq -r '.classification' "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json"
  [ "$output" = "needs-guidance" ]
}

@test "verifier and promotion require policy traceability for v3 issues" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo_base "$TEST_TMPDIR/repo"
  write_v3_plan "$TEST_TMPDIR/repo"
  mkdir -p "$TEST_TMPDIR/repo/.autospec/state/work-items/123"
  cat > "$TEST_TMPDIR/repo/.autospec/state/work-items/123/implementation-packet.json" <<'JSON'
{"structured_policy_context":{"rule_ids":["testing.playwright.required_for_web"],"quality_gate_ids":["testing.playwright.required_for_web.gate_1"],"acceptance_criteria":["Playwright is configured."]}}
JSON
  cat > "$TEST_TMPDIR/repo/.autospec/state/implementation-packet.md" <<'MD'
# Packet
## Structured Policy Context
## Rule IDs
testing.playwright.required_for_web
## Quality Gates
testing.playwright.required_for_web.gate_1
## Structured Acceptance Criteria
- [ ] Playwright is configured.
MD
  cat > "$TEST_TMPDIR/repo/.autospec/reports/worker-risk-classification.json" <<'JSON'
{"classification":"metadata-only","processed_issue_id":"001-testing-playwright"}
JSON
  cat > "$TEST_TMPDIR/repo/.autospec/reports/worker-diff-review.json" <<'JSON'
{"files_changed":[{"path":".autospec/state/rule-check-results.json"}],"forbidden_path_check":{"passed":true},"patch_budget":{"passed":true},"test_docs_metadata_change_check":{"test_files":[]}}
JSON
  cat > "$TEST_TMPDIR/repo/.autospec/reports/worker-validation.json" <<'JSON'
{"focused":[{"command":"bash scripts/autospec-constitution-audit.sh","exit_code":0}]}
JSON

  run bash "$VERIFY" --repo-root "$TEST_TMPDIR/repo" --dry-run --work-item "$TEST_TMPDIR/repo/.autospec/state/work-items/123"
  [ "$status" -eq 0 ]
  grep -q 'Policy Traceability' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.md"
  run jq -r '.policy_traceability.status' "$TEST_TMPDIR/repo/.autospec/reports/verifier-report.json"
  [ "$output" = "pass" ]

  run bash "$PROMOTE" --repo-root "$TEST_TMPDIR/repo" --dry-run --pr 7
  [ "$status" -eq 0 ]
  run jq -r '.policy_traceability_status' "$TEST_TMPDIR/repo/.autospec/reports/promotion-plan.json"
  [ "$output" = "pass" ]
}

@test "audit-to-backlog dry-run plans v3 publishing and status shows structured backlog" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_repo_base "$TEST_TMPDIR/repo"
  write_v3_plan "$TEST_TMPDIR/repo"

  run bash "$AUDIT_TO_BACKLOG" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/audit-to-backlog-plan.md" ]
  [ ! -f "$TEST_TMPDIR/repo/.autospec/state/published-issues.json" ]
  run bash "$STATUS" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  grep -q 'Structured Policy Backlog' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-status.md"
  grep -q 'Required failures' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-status.md"
}

@test "legacy v1 plan remains publishable when v3 and v2 are absent" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/reports" "$TEST_TMPDIR/repo/.autospec/backlog/issues" "$TEST_TMPDIR/repo/.autospec/state"
  cat > "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json" <<'JSON'
{"issues":[{"issue_id":"001-legacy","title":"docs: legacy","draft_path":".autospec/backlog/issues/001-legacy.md","suggested_labels":["autospec:documentation"],"source_gap":{}}]}
JSON
  printf '# docs: legacy\n' > "$TEST_TMPDIR/repo/.autospec/backlog/issues/001-legacy.md"

  run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --dry-run
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-plan.json" ]
  run jq -r '.plan_version' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-plan.json"
  [ "$output" = "v1" ]
}

#!/usr/bin/env bats
# tests/unit/test_issue_planning.bats — dry-run backlog and bot state planning.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  PLAN="$REPO_ROOT/scripts/autospec-plan-issues.sh"
  BOT_STATE="$REPO_ROOT/scripts/autospec-bot-state-init.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-issue-planning-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_reports() {
  local repo="$1"
  mkdir -p "$repo/.autospec/reports"
  cat > "$repo/.autospec/reports/metadata-discovery.json" <<'JSON'
{
  "version": 1,
  "facts": {
    "repo_name": {"value": "sample-service", "confidence": 1.0, "evidence": ["repository root directory name"]},
    "product_purpose": {"value": "Sample Service processes customer events.", "confidence": 0.7, "evidence": ["README.md"]}
  },
  "coverage": {
    "docs": {"status": "present", "evidence": ["README.md", "docs/USER_MANUAL.md"]},
    "tests": {"status": "missing", "evidence": ["no tests/ or test-named files found"]}
  },
  "indicators": {
    "ui": {"value": false, "confidence": 0.2, "evidence": ["no UI indicators found"]},
    "api": {"value": true, "confidence": 0.8, "evidence": ["api/openapi.yml"]}
  }
}
JSON
  cat > "$repo/.autospec/reports/baseline-composition.json" <<'JSON'
{
  "version": 1,
  "status": "pass",
  "baselines": {"requested_profiles": ["web"]},
  "composed": {
    "capabilities": [
      {"id": "documentation", "profile": "web"},
      {"id": "testing", "profile": "web"},
      {"id": "ui", "profile": "web"}
    ],
    "requirements": []
  }
}
JSON
  cat > "$repo/.autospec/reports/baseline-gap-analysis.json" <<'JSON'
{
  "version": 1,
  "status": "fail",
  "matrix": [
    {
      "feature_family": "web",
      "capability": "documentation",
      "status": "present",
      "confidence": 0.9,
      "evidence": ["README.md"],
      "priority": "none",
      "suggested_issue_title": ""
    },
    {
      "feature_family": "web",
      "capability": "testing",
      "status": "missing",
      "confidence": 0.8,
      "evidence": ["no tests/ or test-named files found"],
      "priority": "high",
      "suggested_issue_title": "test: add baseline testing evidence"
    },
    {
      "feature_family": "web",
      "capability": "ui",
      "status": "missing",
      "confidence": 0.75,
      "evidence": ["no UI indicators found"],
      "priority": "high",
      "suggested_issue_title": "feat: add UI baseline evidence"
    }
  ]
}
JSON
  cat > "$repo/.autospec/reports/constitutional-gap-report.json" <<'JSON'
{
  "version": 1,
  "status": "fail",
  "sections": {
    "testing_gaps": {
      "status": "gap",
      "summary": "Testing evidence is missing or incomplete.",
      "evidence": ["testing"],
      "suggested_issues": [
        {
          "title": "test: add baseline testing evidence",
          "acceptance_criteria": [
            "A tests/ directory or recognized test files exist.",
            "Baseline gap analysis no longer reports testing gaps."
          ]
        }
      ]
    },
    "ui_ux_gaps": {
      "status": "gap",
      "summary": "UI/UX baseline evidence is missing or incomplete.",
      "evidence": ["ui"],
      "suggested_issues": [
        {
          "title": "feat: add UI baseline evidence",
          "acceptance_criteria": [
            "UI entry points or components are discoverable.",
            "Baseline gap analysis reports UI capability as present or intentionally opted out."
          ]
        }
      ]
    }
  },
  "next_recommended_issues": [
    {
      "title": "test: add baseline testing evidence",
      "acceptance_criteria": [
        "A tests/ directory or recognized test files exist.",
        "Baseline gap analysis no longer reports testing gaps."
      ]
    },
    {
      "title": "feat: add UI baseline evidence",
      "acceptance_criteria": [
        "UI entry points or components are discoverable.",
        "Baseline gap analysis reports UI capability as present or intentionally opted out."
      ]
    }
  ]
}
JSON
}

@test "gap-to-issue planner writes deterministic issue plan and backlog drafts" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_reports "$TEST_TMPDIR/repo"

  run bash "$PLAN" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 0 ]
  [[ "$output" == *"issue planning: PASS"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/backlog/issues/001-test-add-baseline-testing-evidence.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/backlog/issues/002-feat-add-ui-baseline-evidence.md" ]
  run jq -r '.issues[0].title' "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json"
  [ "$output" = "test: add baseline testing evidence" ]
  run jq -r '.issues[0].source_gap.capability' "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json"
  [ "$output" = "testing" ]
  grep -q '## Acceptance criteria' "$TEST_TMPDIR/repo/.autospec/backlog/issues/001-test-add-baseline-testing-evidence.md"
  grep -q 'bash scripts/validate.sh' "$TEST_TMPDIR/repo/.autospec/backlog/issues/001-test-add-baseline-testing-evidence.md"
}

@test "issue planner is idempotent and clears stale generated drafts" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/backlog/issues"
  write_reports "$TEST_TMPDIR/repo"
  printf '# stale\n' > "$TEST_TMPDIR/repo/.autospec/backlog/issues/999-stale.md"

  bash "$PLAN" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  [ ! -f "$TEST_TMPDIR/repo/.autospec/backlog/issues/999-stale.md" ]
  run bash -c "find '$TEST_TMPDIR/repo/.autospec/backlog/issues' -maxdepth 1 -type f -name '*.md' | wc -l"
  [ "${output//[[:space:]]/}" = "2" ]
}

@test "issue planner writes actionable metadata fields for every draft" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_reports "$TEST_TMPDIR/repo"

  bash "$PLAN" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  run jq -r '.issues[] | select(.risk=="Medium") | .suggested_labels[]' "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json"
  [[ "$output" == *"autospec:managed"* ]]
  [[ "$output" == *"autospec:discovered"* ]]
  run jq -r '.issues[] | .metadata_files_expected_to_change[]' "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json"
  [[ "$output" == *".autospec/reports/baseline-gap-analysis.json"* ]]
}

@test "bot state initializer writes inert local control-plane model" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_reports "$TEST_TMPDIR/repo"
  bash "$PLAN" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  run bash "$BOT_STATE" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 0 ]
  [[ "$output" == *"bot state init: PASS"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/bot-control-plane.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/autonomous-backlog.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/templates/autonomous-issue.md" ]
  run jq -r '.mode' "$TEST_TMPDIR/repo/.autospec/state/bot-control-plane.json"
  [ "$output" = "dry_run" ]
  run jq -r '.write_permissions.github' "$TEST_TMPDIR/repo/.autospec/state/bot-control-plane.json"
  [ "$output" = "false" ]
  run jq -r '.queue.ready[0].title' "$TEST_TMPDIR/repo/.autospec/state/autonomous-backlog.json"
  [ "$output" = "test: add baseline testing evidence" ]
}

@test "bot state initializer does not overwrite manual state unless requested" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/state" "$TEST_TMPDIR/repo/.autospec/reports"
  printf '{"mode":"manual"}\n' > "$TEST_TMPDIR/repo/.autospec/state/bot-control-plane.json"
  printf '{"version":1,"issues":[]}\n' > "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json"

  run bash "$BOT_STATE" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 1 ]
  [[ "$output" == *"bot-control-plane.json already exists"* ]]
  run jq -r '.mode' "$TEST_TMPDIR/repo/.autospec/state/bot-control-plane.json"
  [ "$output" = "manual" ]
}

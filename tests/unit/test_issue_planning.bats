#!/usr/bin/env bats
# tests/unit/test_issue_planning.bats — dry-run backlog and bot state planning.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  PLAN="$REPO_ROOT/scripts/autospec-plan-issues.sh"
  BOT_STATE="$REPO_ROOT/scripts/autospec-bot-state-init.sh"
  DRY_RUN="$REPO_ROOT/scripts/autospec-autonomy-dry-run.sh"
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
  grep -q 'autospec validate' "$TEST_TMPDIR/repo/.autospec/backlog/issues/001-test-add-baseline-testing-evidence.md"
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

@test "issue planner orders by planning bucket and generates dependencies" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/reports"
  cat > "$TEST_TMPDIR/repo/.autospec/reports/metadata-discovery.json" <<'JSON'
{"version":1,"facts":{"repo_name":{"value":"planner-fixture","confidence":1.0,"evidence":["fixture"]}}}
JSON
  cat > "$TEST_TMPDIR/repo/.autospec/reports/baseline-composition.json" <<'JSON'
{"version":1,"status":"pass","baselines":{"requested_profiles":["web","ai-platform","ops"]},"composed":{"capabilities":[]}}
JSON
  cat > "$TEST_TMPDIR/repo/.autospec/reports/baseline-gap-analysis.json" <<'JSON'
{
  "version": 1,
  "status": "fail",
  "matrix": [
    {"feature_family":"ops","capability":"operations","status":"missing","confidence":0.7,"evidence":["no CI"],"priority":"high","suggested_issue_title":"chore: add operations diagnostics"},
    {"feature_family":"web","capability":"documentation","status":"missing","confidence":0.7,"evidence":["no docs UI"],"priority":"high","suggested_issue_title":"docs: add documentation center"},
    {"feature_family":"ai-platform","capability":"ai assistant","status":"missing","confidence":0.6,"evidence":["no AI"],"priority":"high","suggested_issue_title":"feat: add AI assistant"},
    {"feature_family":"architecture","capability":"architecture migration","status":"missing","confidence":0.5,"evidence":["legacy boundary"],"priority":"high","suggested_issue_title":"refactor: migrate architecture boundary"},
    {"feature_family":"web","capability":"settings ui","status":"missing","confidence":0.7,"evidence":["no settings"],"priority":"low","suggested_issue_title":"feat: add settings area"},
    {"feature_family":"baseline","capability":"metadata configuration","status":"missing","confidence":0.8,"evidence":["missing config"],"priority":"medium","suggested_issue_title":"chore: add autospec metadata config"},
    {"feature_family":"web","capability":"testing","status":"missing","confidence":0.8,"evidence":["no tests"],"priority":"high","suggested_issue_title":"test: add validation coverage"}
  ]
}
JSON
  cat > "$TEST_TMPDIR/repo/.autospec/reports/constitutional-gap-report.json" <<'JSON'
{"version":1,"status":"fail","sections":{},"next_recommended_issues":[]}
JSON

  bash "$PLAN" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  run jq -r '.issues[].title' "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json"
  [ "$output" = $'chore: add autospec metadata config\ntest: add validation coverage\ndocs: add documentation center\nfeat: add settings area\nfeat: add AI assistant\nchore: add operations diagnostics\nrefactor: migrate architecture boundary' ]
  run jq -r '.issues[] | select(.title=="feat: add AI assistant") | .depends_on[]' "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json"
  [[ "$output" == *"001-chore-add-autospec-metadata-config"* ]]
  [[ "$output" == *"003-docs-add-documentation-center"* ]]
  [[ "$output" == *"004-feat-add-settings-area"* ]]
  run jq -r '.issues[] | select(.title=="refactor: migrate architecture boundary") | .blocked_reason' "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json"
  [[ "$output" == *"Depends on lower-risk backlog items"* ]]
  grep -q '001-chore-add-autospec-metadata-config' "$TEST_TMPDIR/repo/.autospec/backlog/issues/005-feat-add-ai-assistant.md"
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

@test "bot state initializer writes control label taxonomy" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_reports "$TEST_TMPDIR/repo"
  bash "$PLAN" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  bash "$BOT_STATE" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  [ -f "$TEST_TMPDIR/repo/.autospec/state/control-labels.yml" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/control-labels.md" ]
  run grep -c '^  autospec:' "$TEST_TMPDIR/repo/.autospec/state/control-labels.yml"
  [ "$output" = "14" ]
  for label in \
    autospec:managed \
    autospec:discovered \
    autospec:active \
    autospec:paused \
    autospec:blocked \
    autospec:stuck \
    autospec:needs-guidance \
    autospec:guidance-provided \
    autospec:resume \
    autospec:needs-review \
    autospec:architecture \
    autospec:risk-high \
    autospec:self-improvement \
    autospec:follow-up
  do
    grep -q "  $label:" "$TEST_TMPDIR/repo/.autospec/state/control-labels.yml"
    grep -q "| \`$label\` |" "$TEST_TMPDIR/repo/.autospec/reports/control-labels.md"
  done
  run python3 - "$TEST_TMPDIR/repo/.autospec/state/control-labels.yml" <<'PY'
import sys
import yaml
data = yaml.safe_load(open(sys.argv[1], encoding="utf-8"))
required = ["purpose", "may_apply", "may_remove", "compatible_labels", "incompatible_labels", "state_machine_effect"]
missing = []
for label, spec in data["labels"].items():
    for field in required:
        if field not in spec:
            missing.append(f"{label}:{field}")
print("\n".join(missing))
sys.exit(1 if missing else 0)
PY
  [ "$status" -eq 0 ]
}

@test "bot state initializer writes bot state machine" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_reports "$TEST_TMPDIR/repo"
  bash "$PLAN" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  bash "$BOT_STATE" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  [ -f "$TEST_TMPDIR/repo/.autospec/state/bot-state-machine.yml" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/bot-state-machine.md" ]
  run python3 - "$TEST_TMPDIR/repo/.autospec/state/bot-state-machine.yml" <<'PY'
import sys
import yaml
data = yaml.safe_load(open(sys.argv[1], encoding="utf-8"))
expected_states = [
    "candidate",
    "claimed",
    "active",
    "paused",
    "blocked",
    "stuck",
    "guidance-provided",
    "ready-to-resume",
    "completed",
    "cancelled",
]
expected_transitions = [
    ("candidate", "claimed"),
    ("claimed", "active"),
    ("active", "completed"),
    ("active", "stuck"),
    ("active", "blocked"),
    ("active", "paused"),
    ("stuck", "guidance-provided"),
    ("guidance-provided", "ready-to-resume"),
    ("ready-to-resume", "claimed"),
    ("blocked", "ready-to-resume"),
    ("paused", "ready-to-resume"),
]
required = ["required_labels_before", "labels_to_add", "labels_to_remove", "required_evidence", "human_action_required"]
states = data.get("states", [])
transitions = {(item.get("from"), item.get("to")): item for item in data.get("transitions", [])}
missing = []
for state in expected_states:
    if state not in states:
        missing.append(f"state:{state}")
for edge in expected_transitions:
    item = transitions.get(edge)
    if not item:
        missing.append(f"transition:{edge[0]}->{edge[1]}")
        continue
    for field in required:
        if field not in item:
            missing.append(f"{edge[0]}->{edge[1]}:{field}")
if transitions[("stuck", "guidance-provided")]["human_action_required"] is not True:
    missing.append("stuck->guidance-provided:human_action_required")
if "autospec:active" not in transitions[("claimed", "active")]["labels_to_add"]:
    missing.append("claimed->active:adds-active")
print("\n".join(missing))
sys.exit(1 if missing else 0)
PY
  [ "$status" -eq 0 ]
  grep -q '| `candidate` | `claimed` |' "$TEST_TMPDIR/repo/.autospec/reports/bot-state-machine.md"
  grep -q '| `stuck` | `guidance-provided` |' "$TEST_TMPDIR/repo/.autospec/reports/bot-state-machine.md"
}

@test "bot state initializer writes stuck issue template" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_reports "$TEST_TMPDIR/repo"
  bash "$PLAN" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  bash "$BOT_STATE" --repo-root "$TEST_TMPDIR/repo" >/dev/null

  [ -f "$TEST_TMPDIR/repo/.autospec/templates/stuck-issue.md" ]
  grep -q '^# bot stuck: <task>$' "$TEST_TMPDIR/repo/.autospec/templates/stuck-issue.md"
  grep -q '^## Bot stuck$' "$TEST_TMPDIR/repo/.autospec/templates/stuck-issue.md"
  grep -q '^## What I was trying to do$' "$TEST_TMPDIR/repo/.autospec/templates/stuck-issue.md"
  grep -q '^## What I tried$' "$TEST_TMPDIR/repo/.autospec/templates/stuck-issue.md"
  grep -q '^## Why I cannot proceed safely$' "$TEST_TMPDIR/repo/.autospec/templates/stuck-issue.md"
  grep -q '^## Exact guidance needed$' "$TEST_TMPDIR/repo/.autospec/templates/stuck-issue.md"
  grep -q '^## Options I considered$' "$TEST_TMPDIR/repo/.autospec/templates/stuck-issue.md"
  grep -q '^## Recommended human action$' "$TEST_TMPDIR/repo/.autospec/templates/stuck-issue.md"
  grep -q '^## Resume criteria$' "$TEST_TMPDIR/repo/.autospec/templates/stuck-issue.md"
  grep -q 'autospec:guidance-provided` or `autospec:resume` applied' "$TEST_TMPDIR/repo/.autospec/templates/stuck-issue.md"
}

@test "combined autonomy dry-run writes summary reports" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_reports "$TEST_TMPDIR/repo"

  run bash "$DRY_RUN" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 0 ]
  [[ "$output" == *"autonomy dry-run: PASS"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/autonomy-dry-run.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/autonomy-dry-run.md" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/issue-plan.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/control-labels.yml" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/bot-state-machine.yml" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/templates/stuck-issue.md" ]
  run jq -r '.status' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-dry-run.json"
  [ "$output" = "pass" ]
  run jq -r '.side_effects.github_api_calls' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-dry-run.json"
  [ "$output" = "false" ]
  grep -q '## Detected App Type / Stack' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-dry-run.md"
  grep -q '## Proposed Issue Backlog' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-dry-run.md"
  grep -q '## Stuck / Guidance Protocol' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-dry-run.md"
  grep -q 'bash scripts/autospec-autonomy-dry-run.sh --repo-root' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-dry-run.md"
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

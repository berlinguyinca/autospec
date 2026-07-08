#!/usr/bin/env bats
# tests/unit/test_autonomy_pipeline.bats — bounded end-to-end autonomy pipeline.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  PROMOTE="$REPO_ROOT/scripts/autospec-promote-pr.sh"
  REMEDIATE="$REPO_ROOT/scripts/autospec-plan-remediation.sh"
  WORKER_ONE="$REPO_ROOT/scripts/autospec-worker-one.sh"
  STUCK_PUBLISH="$REPO_ROOT/scripts/autospec-publish-stuck.sh"
  GUIDANCE_SYNC="$REPO_ROOT/scripts/autospec-sync-guidance.sh"
  SUPERVISOR="$REPO_ROOT/scripts/autospec-supervisor-cycle.sh"
  STATUS="$REPO_ROOT/scripts/autospec-autonomy-status.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-pipeline-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_base_state() {
  local repo="$1"
  mkdir -p "$repo/.autospec/reports" "$repo/.autospec/state/verifications" "$repo/.autospec/state/work-items/1" "$repo/.autospec/templates"
  cat > "$repo/.autospec/reports/issue-plan.json" <<'JSON'
{"version":1,"issues":[{"issue_id":"001-fix-report-formatting","title":"fix: improve report formatting helper","risk":"Low","suggested_labels":["autospec:managed"],"draft_path":".autospec/backlog/issues/001.md"}]}
JSON
  cat > "$repo/.autospec/state/published-issues.json" <<'JSON'
{"schema":1,"repo":"example/repo","issues":[{"local_issue_id":"001-fix-report-formatting","github_issue_number":1,"github_issue_url":"https://github.com/example/repo/issues/1","state":"open","labels":["autospec:managed"]}]}
JSON
  cat > "$repo/.autospec/state/control-labels.yml" <<'YAML'
labels:
  autospec:verified: {purpose: verifier passed}
  autospec:needs-human-review: {purpose: human review ready}
  autospec:needs-changes: {purpose: changes needed}
  autospec:verification-failed: {purpose: verifier failed}
  autospec:stuck: {purpose: stuck}
  autospec:needs-guidance: {purpose: needs guidance}
  autospec:managed: {purpose: managed}
YAML
  cat > "$repo/.autospec/state/bot-runs.json" <<'JSON'
{"schema":1,"runs":[]}
JSON
  cat > "$repo/.autospec/reports/worker-risk-classification.json" <<'JSON'
{"version":1,"processed_issue_id":"001-fix-report-formatting","classification":"low-risk-code"}
JSON
  cat > "$repo/.autospec/reports/worker-diff-review.json" <<'JSON'
{"version":1,"patch_budget":{"passed":true,"failures":[]},"forbidden_path_check":{"passed":true,"matches":[]},"pr_creation_allowed":true}
JSON
  cat > "$repo/.autospec/reports/worker-result.json" <<'JSON'
{"version":1,"issue_id":"001-fix-report-formatting","classification":"low-risk-code","pr_creation_allowed":true}
JSON
}

write_verifier() {
  local repo="$1"
  local verdict="$2"
  local extra_dimension="${3:-}"
  cat > "$repo/.autospec/reports/verifier-report.json" <<JSON
{
  "version": 1,
  "verdict": "$verdict",
  "source": {"pr":"7","issue":"1","processed_issue_id":"001-fix-report-formatting"},
  "dimensions": [
    {"dimension":"issue_alignment","status":"pass","summary":"aligned","evidence":["001"],"required_action":""},
    {"dimension":"validation_evidence","status":"pass","summary":"validation captured","evidence":["bats"],"required_action":""},
    {"dimension":"patch_budget","status":"pass","summary":"budget pass","evidence":[],"required_action":""},
    {"dimension":"forbidden_paths","status":"pass","summary":"none","evidence":[],"required_action":""},
    {"dimension":"pr_body_completeness","status":"pass","summary":"complete","evidence":[],"required_action":""}
    $extra_dimension
  ],
  "required_actions": []
}
JSON
  cp "$repo/.autospec/reports/verifier-report.json" "$repo/.autospec/state/verifications/pr-7.json"
}

install_gh_stub() {
  local bin="$1"
  local log="$2"
  mkdir -p "$bin"
  cat > "$bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_STUB_LOG"
if [ "$1" = "--repo" ]; then shift 2; fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  printf '{"number":7,"title":"fix: improve report formatting helper","isDraft":true,"headRefName":"autospec/worker-1","labels":[{"name":"autospec:managed"}]}\n'
  exit 0
fi
if [ "$1" = "pr" ] && { [ "$2" = "edit" ] || [ "$2" = "ready" ] || [ "$2" = "comment" ]; }; then
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then printf '[]\n'; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "create" ]; then printf 'https://github.com/example/repo/issues/99\n'; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  printf '{"number":99,"title":"bot stuck","state":"OPEN","labels":[{"name":"autospec:guidance-provided"},{"name":"autospec:resume"}],"comments":[{"body":"try smaller patch","createdAt":"2026-06-28T00:00:00Z"}]}\n'
  exit 0
fi
printf 'unexpected gh call: %s\n' "$*" >&2
exit 1
SH
  chmod +x "$bin/gh"
  : > "$log"
}

@test "promotion gate allows verifier pass and confirm applies labels without approval or merge" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_base_state "$TEST_TMPDIR/repo"
  write_verifier "$TEST_TMPDIR/repo" "pass"
  install_gh_stub "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  run bash "$PROMOTE" --repo-root "$TEST_TMPDIR/repo" --dry-run --pr 7 --repo example/repo
  [ "$status" -eq 0 ]
  run jq -r '.promotion_allowed' "$TEST_TMPDIR/repo/.autospec/reports/promotion-plan.json"
  [ "$output" = "true" ]

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PROMOTE" --repo-root "$TEST_TMPDIR/repo" --confirm --pr 7 --repo example/repo
  [ "$status" -eq 0 ]
  grep -q 'issue edit 7 --add-label autospec:needs-human-review' "$TEST_TMPDIR/gh.log"
  ! grep -Eq 'pr (merge|review)' "$TEST_TMPDIR/gh.log"
}

@test "promotion blocks verifier failures missing verifier and high risk by default" {
  mkdir -p "$TEST_TMPDIR/fail" "$TEST_TMPDIR/missing" "$TEST_TMPDIR/high"
  write_base_state "$TEST_TMPDIR/fail"; write_verifier "$TEST_TMPDIR/fail" "needs_changes"
  run bash "$PROMOTE" --repo-root "$TEST_TMPDIR/fail" --dry-run --pr 7
  [ "$status" -eq 1 ]
  run jq -r '.promotion_allowed' "$TEST_TMPDIR/fail/.autospec/reports/promotion-plan.json"
  [ "$output" = "false" ]

  write_base_state "$TEST_TMPDIR/missing"
  run bash "$PROMOTE" --repo-root "$TEST_TMPDIR/missing" --dry-run --pr 7
  [ "$status" -eq 1 ]

  write_base_state "$TEST_TMPDIR/high"; write_verifier "$TEST_TMPDIR/high" "pass"
  cat > "$TEST_TMPDIR/high/.autospec/reports/worker-risk-classification.json" <<'JSON'
{"version":1,"processed_issue_id":"001-fix-report-formatting","classification":"high-risk-code"}
JSON
  run bash "$PROMOTE" --repo-root "$TEST_TMPDIR/high" --dry-run --pr 7
  [ "$status" -eq 1 ]
  grep -q 'high-risk' "$TEST_TMPDIR/high/.autospec/reports/promotion-plan.md"
}

@test "remediation planner groups findings and worker remediation is gated" {
  mkdir -p "$TEST_TMPDIR/repo/.git"
  write_base_state "$TEST_TMPDIR/repo"
  write_verifier "$TEST_TMPDIR/repo" "needs_changes" ',{"dimension":"validation_evidence","status":"fail","summary":"missing validation","evidence":[],"required_action":"Run focused validation."}'

  run bash "$REMEDIATE" --repo-root "$TEST_TMPDIR/repo" --dry-run --pr 7
  [ "$status" -eq 0 ]
  run jq -r '.groups.required_before_human_review[0].source_dimension' "$TEST_TMPDIR/repo/.autospec/reports/remediation-plan.json"
  [ "$output" = "validation_evidence" ]
  run jq -r '.safe_for_worker_remediation' "$TEST_TMPDIR/repo/.autospec/reports/remediation-plan.json"
  [ "$output" = "true" ]

  run bash "$WORKER_ONE" --repo-root "$TEST_TMPDIR/repo" --dry-run --remediate --pr 7 --branch feature/not-autospec
  [ "$status" -eq 1 ]
  grep -q 'non-autospec branch' "$TEST_TMPDIR/repo/.autospec/reports/worker-remediation-result.md"

  run bash "$WORKER_ONE" --repo-root "$TEST_TMPDIR/repo" --dry-run --remediate --pr 7 --branch autospec/worker-1
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/work-items/1/remediation-result.json" ]

  run bash "$REPO_ROOT/scripts/autospec-worker-v1.sh" --repo-root "$TEST_TMPDIR/repo" --dry-run --remediate --pr 7 --branch autospec/worker-1
  [ "$status" -eq 0 ]
}

@test "stuck publishing is idempotent and guidance sync marks ready-to-resume without resuming" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/state/work-items/1" "$TEST_TMPDIR/repo/.autospec/templates"
  write_base_state "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/repo/.autospec/state/work-items/1/stuck-handoff.md" <<'MD'
# bot stuck: test

## Why worker v1 refused this issue
Needs guidance.
MD
  install_gh_stub "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$STUCK_PUBLISH" --repo-root "$TEST_TMPDIR/repo" --confirm --work-item 1 --repo example/repo
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/stuck-handovers.json" ]
  grep -q 'autospec-stuck-for-issue: 1' "$TEST_TMPDIR/repo/.autospec/reports/stuck-publish-result.md"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$GUIDANCE_SYNC" --repo-root "$TEST_TMPDIR/repo" --confirm --repo example/repo
  [ "$status" -eq 0 ]
  run jq -r '.handovers[0].state' "$TEST_TMPDIR/repo/.autospec/state/stuck-handovers.json"
  [ "$output" = "ready-to-resume" ]
  ! grep -q 'worker-one' "$TEST_TMPDIR/gh.log"
}

@test "supervisor dry-run and confirmed cycle process one issue and route verifier outcomes" {
  mkdir -p "$TEST_TMPDIR/pass" "$TEST_TMPDIR/fail"
  write_base_state "$TEST_TMPDIR/pass"; write_verifier "$TEST_TMPDIR/pass" "pass"
  write_base_state "$TEST_TMPDIR/fail"; write_verifier "$TEST_TMPDIR/fail" "needs_changes" ',{"dimension":"validation_evidence","status":"fail","summary":"missing validation","evidence":[],"required_action":"Run focused validation."}'
  install_gh_stub "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  run bash "$SUPERVISOR" --repo-root "$TEST_TMPDIR/pass" --dry-run --issue 1
  [ "$status" -eq 0 ]
  run jq -r '.planned_issue_count' "$TEST_TMPDIR/pass/.autospec/reports/supervisor-cycle-plan.json"
  [ "$output" = "1" ]

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$SUPERVISOR" --repo-root "$TEST_TMPDIR/pass" --confirm --issue 1 --repo example/repo
  [ "$status" -eq 0 ]
  run jq -r '.outcome' "$TEST_TMPDIR/pass/.autospec/reports/supervisor-cycle-result.json"
  [ "$output" = "promotion_planned" ]
  [ -f "$TEST_TMPDIR/pass/.autospec/state/supervisor-runs.json" ]

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$SUPERVISOR" --repo-root "$TEST_TMPDIR/fail" --confirm --issue 1 --repo example/repo
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/fail/.autospec/reports/remediation-plan.json" ]
}

@test "autonomy status summarizes managed stuck verified and review queues" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/state/promotions" "$TEST_TMPDIR/repo/.autospec/state/verifications"
  write_base_state "$TEST_TMPDIR/repo"
  write_verifier "$TEST_TMPDIR/repo" "pass"
  cat > "$TEST_TMPDIR/repo/.autospec/state/stuck-handovers.json" <<'JSON'
{"schema":1,"handovers":[{"work_item_id":"1","state":"ready-to-resume","stuck_issue_number":99}]}
JSON
  cat > "$TEST_TMPDIR/repo/.autospec/state/promotions/pr-7.json" <<'JSON'
{"version":1,"promotion_allowed":true,"labels_to_add":["autospec:needs-human-review","autospec:verified"]}
JSON

  run bash "$STATUS" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/autonomy-status.md" ]
  grep -q '# Autospec Autonomy Status' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-status.md"
  run jq -r '.summary.managed_issues' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-status.json"
  [ "$output" = "1" ]
}

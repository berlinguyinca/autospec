#!/usr/bin/env bats
# tests/unit/test_local_autonomy_control.bats — local-only multi-cycle autonomy controls.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  LOOP="$REPO_ROOT/scripts/autospec-supervisor-loop.sh"
  BUDGET="$REPO_ROOT/scripts/autospec-autonomy-budget.sh"
  REPEATED="$REPO_ROOT/scripts/autospec-repeated-failures.sh"
  STATUS="$REPO_ROOT/scripts/autospec-autonomy-status.sh"
  RESUME="$REPO_ROOT/scripts/autospec-resume.sh"
  GUIDE="$REPO_ROOT/scripts/autospec-guide-issue.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-local-control-XXXXXX)"
  export HOME="$TEST_TMPDIR/home"
  mkdir -p "$HOME"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_base_state() {
  local repo="$1"
  mkdir -p "$repo/.autospec/reports" "$repo/.autospec/state/verifications" "$repo/.autospec/state/promotions"
  cat > "$repo/.autospec/reports/issue-plan.json" <<'JSON'
{"version":1,"issues":[{"issue_id":"001-local-control","title":"fix: local control","risk":"Low"}]}
JSON
  cat > "$repo/.autospec/state/published-issues.json" <<'JSON'
{"schema":1,"repo":"example/repo","issues":[{"local_issue_id":"001-local-control","github_issue_number":1,"state":"open","labels":["autospec:managed"]}]}
JSON
  cat > "$repo/.autospec/reports/verifier-report.json" <<'JSON'
{"version":1,"verdict":"pass","dimensions":[{"dimension":"validation_evidence","status":"pass","summary":"ok","evidence":["bats"],"required_action":""}]}
JSON
  cp "$repo/.autospec/reports/verifier-report.json" "$repo/.autospec/state/verifications/pr-7.json"
  cat > "$repo/.autospec/state/stuck-handovers.json" <<'JSON'
{"schema":1,"handovers":[]}
JSON
}

install_gh_stub() {
  local bin="$1"
  local log="$2"
  mkdir -p "$bin"
  cat > "$bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_STUB_LOG"
if [ "$1" = "--repo" ]; then shift 2; fi
if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
printf 'unexpected gh call: %s\n' "$*" >&2
exit 1
SH
  chmod +x "$bin/gh"
  : > "$log"
}

@test "supervisor loop dry-run plans bounded local cycles without lock or GitHub Actions" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_base_state "$TEST_TMPDIR/repo"

  run bash "$LOOP" --repo-root "$TEST_TMPDIR/repo" --dry-run --max-cycles 3
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/supervisor-loop-plan.md" ]
  run jq -r '.max_cycles' "$TEST_TMPDIR/repo/.autospec/reports/supervisor-loop-plan.json"
  [ "$output" = "3" ]
  [ ! -e "$TEST_TMPDIR/repo/.autospec/run.lock" ]
  ! find "$TEST_TMPDIR/repo" -path '*/.github/workflows/*' | grep -q .
}

@test "confirmed supervisor loop acquires lock writes session history and releases on success" {
  mkdir -p "$TEST_TMPDIR/repo"
  write_base_state "$TEST_TMPDIR/repo"

  run bash "$LOOP" --repo-root "$TEST_TMPDIR/repo" --confirm --max-cycles 2 --issue 1
  [ "$status" -eq 0 ]
  run jq -r '.completed_cycles' "$TEST_TMPDIR/repo/.autospec/reports/supervisor-loop-result.json"
  [ "$output" = "2" ]
  run jq -r '.stop_reason' "$TEST_TMPDIR/repo/.autospec/reports/supervisor-loop-result.json"
  [ "$output" = "completed_requested_cycles" ]
  run jq -r '.status' "$TEST_TMPDIR/repo/.autospec/state/current-run.json"
  [ "$output" = "completed" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/run-history.json" ]
  [ ! -e "$TEST_TMPDIR/repo/.autospec/run.lock" ]
}

@test "loop refuses active lock reports stale lock recovery and honors stop flag" {
  mkdir -p "$TEST_TMPDIR/locked" "$TEST_TMPDIR/stale" "$TEST_TMPDIR/stopped"
  write_base_state "$TEST_TMPDIR/locked"
  write_base_state "$TEST_TMPDIR/stale"
  write_base_state "$TEST_TMPDIR/stopped"
  mkdir -p "$TEST_TMPDIR/locked/.autospec/run.lock" "$TEST_TMPDIR/stale/.autospec/run.lock" "$HOME/.autospec"
  printf '%s\n' "$$" > "$TEST_TMPDIR/locked/.autospec/run.lock/pid"
  printf '999999\n' > "$TEST_TMPDIR/stale/.autospec/run.lock/pid"
  printf 'graceful\n2026-06-28T00:00:00Z test@host\n' > "$HOME/.autospec/stop.flag"

  run bash "$LOOP" --repo-root "$TEST_TMPDIR/locked" --confirm --max-cycles 1
  [ "$status" -eq 1 ]
  grep -q 'repo_lock_unavailable' "$TEST_TMPDIR/locked/.autospec/reports/supervisor-loop-result.md"

  rm -f "$HOME/.autospec/stop.flag"
  run bash "$LOOP" --repo-root "$TEST_TMPDIR/stale" --confirm --max-cycles 1
  [ "$status" -eq 1 ]
  grep -q 'stale lock' "$TEST_TMPDIR/stale/.autospec/reports/supervisor-loop-result.md"

  rm -rf "$TEST_TMPDIR/stale/.autospec/run.lock"
  printf 'graceful\n2026-06-28T00:00:00Z test@host\n' > "$HOME/.autospec/stop.flag"
  run bash "$LOOP" --repo-root "$TEST_TMPDIR/stopped" --confirm --max-cycles 1
  [ "$status" -eq 1 ]
  run jq -r '.stop_reason' "$TEST_TMPDIR/stopped/.autospec/reports/supervisor-loop-result.json"
  [ "$output" = "stop_flag" ]
}

@test "budget and repeated failures block unsafe loops" {
  mkdir -p "$TEST_TMPDIR/budget/.autospec/state/verifications" "$TEST_TMPDIR/repeated/.autospec/state/verifications"
  write_base_state "$TEST_TMPDIR/budget"
  write_base_state "$TEST_TMPDIR/repeated"
  cat > "$TEST_TMPDIR/budget/.autospec/autospec.yml" <<'YAML'
autonomy:
  budgets:
    max_open_autospec_prs: 0
YAML
  cat > "$TEST_TMPDIR/repeated/.autospec/state/verifications/pr-8.json" <<'JSON'
{"version":1,"verdict":"needs_changes","dimensions":[{"dimension":"validation_evidence","status":"fail","summary":"same validation failed","required_action":"rerun"}]}
JSON
  cat > "$TEST_TMPDIR/repeated/.autospec/state/verifications/pr-9.json" <<'JSON'
{"version":1,"verdict":"needs_changes","dimensions":[{"dimension":"validation_evidence","status":"fail","summary":"same validation failed","required_action":"rerun"}]}
JSON

  run bash "$BUDGET" --repo-root "$TEST_TMPDIR/budget"
  [ "$status" -eq 1 ]
  run jq -r '.overall_status' "$TEST_TMPDIR/budget/.autospec/reports/autonomy-budget.json"
  [ "$output" = "exhausted" ]

  run bash "$LOOP" --repo-root "$TEST_TMPDIR/budget" --confirm --max-cycles 1
  [ "$status" -eq 1 ]
  run jq -r '.stop_reason' "$TEST_TMPDIR/budget/.autospec/reports/supervisor-loop-result.json"
  [ "$output" = "budget_exhausted" ]

  run bash "$REPEATED" --repo-root "$TEST_TMPDIR/repeated" --threshold 2
  [ "$status" -eq 1 ]
  run jq -r '.repeated_failures[0].kind' "$TEST_TMPDIR/repeated/.autospec/reports/repeated-failures.json"
  [ "$output" = "verifier_finding" ]

  run bash "$LOOP" --repo-root "$TEST_TMPDIR/repeated" --confirm --max-cycles 1
  [ "$status" -eq 1 ]
  run jq -r '.stop_reason' "$TEST_TMPDIR/repeated/.autospec/reports/supervisor-loop-result.json"
  [ "$output" = "repeated_failure" ]
}

@test "resume removes stop flag explicitly and does not start work" {
  mkdir -p "$TEST_TMPDIR/repo" "$HOME/.autospec"
  printf 'graceful\n2026-06-28T00:00:00Z test@host\n' > "$HOME/.autospec/stop.flag"

  run bash "$RESUME" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  [ ! -f "$HOME/.autospec/stop.flag" ]
  grep -q 'does not start work' "$TEST_TMPDIR/repo/.autospec/reports/stop-status.md"
  run jq -r '.resume_performed' "$TEST_TMPDIR/repo/.autospec/reports/stop-status.json"
  [ "$output" = "true" ]
}

@test "guidance helper dry-run posts nothing and confirm comments with resume only when requested" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/state"
  cat > "$TEST_TMPDIR/repo/.autospec/state/stuck-handovers.json" <<'JSON'
{"schema":1,"handovers":[{"work_item_id":"1","source_issue_number":"1","stuck_issue_number":99,"state":"needs-guidance"}]}
JSON
  printf 'Use option 2 and keep scope narrow.\n' > "$TEST_TMPDIR/guidance.md"
  install_gh_stub "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$GUIDE" --repo-root "$TEST_TMPDIR/repo" --dry-run --stuck 99 --message-file "$TEST_TMPDIR/guidance.md"
  [ "$status" -eq 0 ]
  [ ! -s "$TEST_TMPDIR/gh.log" ]
  grep -q 'Use option 2' "$TEST_TMPDIR/repo/.autospec/reports/guidance-post-plan.md"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$GUIDE" --repo-root "$TEST_TMPDIR/repo" --confirm --stuck 99 --message-file "$TEST_TMPDIR/guidance.md" --resume
  [ "$status" -eq 0 ]
  grep -q 'issue comment 99' "$TEST_TMPDIR/gh.log"
  grep -q 'issue edit 99 --add-label autospec:guidance-provided' "$TEST_TMPDIR/gh.log"
  grep -q 'issue edit 99 --add-label autospec:resume' "$TEST_TMPDIR/gh.log"
  run jq -r '.handovers[0].state' "$TEST_TMPDIR/repo/.autospec/state/stuck-handovers.json"
  [ "$output" = "ready-to-resume" ]
}

@test "autospec-guide skill exists references actual commands and forbids unsafe flows" {
  [ -f "$REPO_ROOT/skills/autospec-guide/SKILL.md" ]
  [ -f "$REPO_ROOT/skills/autospec-guide/codex/prompt.md" ]
  [ -f "$REPO_ROOT/skills/autospec-guide/opencode/agent.md" ]
  [ -f "$REPO_ROOT/skills/autospec-guide/README.md" ]
  [ -f "$REPO_ROOT/skills/autospec-guide/install.sh" ]
  [ -f "$REPO_ROOT/skills/autospec-guide/uninstall.sh" ]
  grep -q 'scripts/autospec-supervisor-loop.sh' "$REPO_ROOT/skills/autospec-guide/SKILL.md"
  grep -q 'scripts/autospec-resume.sh' "$REPO_ROOT/skills/autospec-guide/SKILL.md"
  grep -q 'never merge PRs' "$REPO_ROOT/skills/autospec-guide/SKILL.md"
  grep -q 'never approve PRs' "$REPO_ROOT/skills/autospec-guide/SKILL.md"
  diff <(awk '/^---$/{c++; next} c>=2' "$REPO_ROOT/skills/autospec-guide/SKILL.md") "$REPO_ROOT/skills/autospec-guide/codex/prompt.md"
  diff <(awk '/^---$/{c++; next} c>=2' "$REPO_ROOT/skills/autospec-guide/SKILL.md") <(awk '/^---$/{c++; next} c>=2' "$REPO_ROOT/skills/autospec-guide/opencode/agent.md")
}

@test "autonomy status includes lock stop budget repeated failures and guide commands" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/run.lock" "$HOME/.autospec"
  write_base_state "$TEST_TMPDIR/repo"
  printf '%s\n' "$$" > "$TEST_TMPDIR/repo/.autospec/run.lock/pid"
  printf 'graceful\n2026-06-28T00:00:00Z test@host\n' > "$HOME/.autospec/stop.flag"
  cat > "$TEST_TMPDIR/repo/.autospec/reports/autonomy-budget.json" <<'JSON'
{"version":1,"overall_status":"ok","budgets":[]}
JSON
  cat > "$TEST_TMPDIR/repo/.autospec/reports/repeated-failures.json" <<'JSON'
{"version":1,"has_repeated_failures":false,"repeated_failures":[]}
JSON

  run bash "$STATUS" --repo-root "$TEST_TMPDIR/repo"
  [ "$status" -eq 0 ]
  grep -q 'Loop Readiness' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-status.md"
  grep -q 'autospec-guide' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-status.md"
  run jq -r '.summary.locked' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-status.json"
  [ "$output" = "true" ]
  run jq -r '.summary.stopped' "$TEST_TMPDIR/repo/.autospec/reports/autonomy-status.json"
  [ "$output" = "true" ]
}

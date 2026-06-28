#!/usr/bin/env bats
# tests/unit/test_github_publishing.bats — guarded GitHub publishing with stubbed gh.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  PLAN="$REPO_ROOT/scripts/autospec-plan-issues.sh"
  BOT_STATE="$REPO_ROOT/scripts/autospec-bot-state-init.sh"
  ENSURE_LABELS="$REPO_ROOT/scripts/autospec-ensure-labels.sh"
  PUBLISH="$REPO_ROOT/scripts/autospec-publish-issues.sh"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-github-publishing-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

write_reports() {
  local repo="$1"
  mkdir -p "$repo/.autospec/reports"
  cat > "$repo/.autospec/reports/metadata-discovery.json" <<'JSON'
{"version":1,"facts":{"repo_name":{"value":"sample-service","confidence":1.0,"evidence":["fixture"]}}}
JSON
  cat > "$repo/.autospec/reports/baseline-composition.json" <<'JSON'
{"version":1,"status":"pass","baselines":{"requested_profiles":["web"]},"composed":{"capabilities":[]}}
JSON
  cat > "$repo/.autospec/reports/baseline-gap-analysis.json" <<'JSON'
{
  "version": 1,
  "status": "fail",
  "matrix": [
    {"feature_family":"web","capability":"testing","status":"missing","confidence":0.8,"evidence":["no tests"],"priority":"high","suggested_issue_title":"test: add baseline testing evidence"},
    {"feature_family":"web","capability":"ui","status":"missing","confidence":0.7,"evidence":["no UI"],"priority":"high","suggested_issue_title":"feat: add UI baseline evidence"}
  ]
}
JSON
  cat > "$repo/.autospec/reports/constitutional-gap-report.json" <<'JSON'
{
  "version": 1,
  "status": "fail",
  "sections": {},
  "next_recommended_issues": [
    {"title":"test: add baseline testing evidence","acceptance_criteria":["A tests/ directory exists."]},
    {"title":"feat: add UI baseline evidence","acceptance_criteria":["UI entry points are discoverable."]}
  ]
}
JSON
}

prepare_backlog() {
  local repo="$1"
  write_reports "$repo"
  bash "$PLAN" --repo-root "$repo" >/dev/null
  bash "$BOT_STATE" --repo-root "$repo" >/dev/null
}

install_failing_gh() {
  local bin="$1"
  mkdir -p "$bin"
  cat > "$bin/gh" <<'SH'
#!/usr/bin/env bash
printf 'unexpected gh call: %s\n' "$*" >&2
exit 99
SH
  chmod +x "$bin/gh"
}

install_recording_gh() {
  local bin="$1"
  local log="$2"
  mkdir -p "$bin"
  cat > "$bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_STUB_LOG"
if [ "$1" = "issue" ] && [ "$2" = "create" ]; then
  count_file="${GH_STUB_LOG}.create-count"
  count=0
  [ -f "$count_file" ] && count="$(cat "$count_file")"
  count=$((count + 1))
  printf '%s' "$count" > "$count_file"
  printf 'https://github.com/example/repo/issues/%s\n' "$count"
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1" = "label" ] && [ "$2" = "create" ]; then
  exit 0
fi
exit 0
SH
  chmod +x "$bin/gh"
  : > "$log"
}

install_permission_denied_gh() {
  local bin="$1"
  local log="$2"
  mkdir -p "$bin"
  cat > "$bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_STUB_LOG"
if [ "$1" = "label" ] && [ "$2" = "create" ]; then
  printf 'HTTP 403: Resource not accessible by integration\n' >&2
  exit 1
fi
exit 0
SH
  chmod +x "$bin/gh"
  : > "$log"
}

@test "label provisioning dry-run does not call gh and writes reports" {
  mkdir -p "$TEST_TMPDIR/repo"
  prepare_backlog "$TEST_TMPDIR/repo"
  install_failing_gh "$TEST_TMPDIR/bin"

  PATH="$TEST_TMPDIR/bin:$PATH" run bash "$ENSURE_LABELS" --repo-root "$TEST_TMPDIR/repo" --dry-run

  [ "$status" -eq 0 ]
  [[ "$output" == *"label provisioning: DRY-RUN"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-label-plan.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-label-plan.md" ]
  run jq -r '.mode' "$TEST_TMPDIR/repo/.autospec/reports/github-label-plan.json"
  [ "$output" = "dry_run" ]
  run jq -r '.labels[0] | [.name,.description,.color,.purpose,.state_machine_effect] | length' \
    "$TEST_TMPDIR/repo/.autospec/reports/github-label-plan.json"
  [ "$output" = "5" ]
}

@test "label provisioning confirm calls gh label create for taxonomy labels" {
  mkdir -p "$TEST_TMPDIR/repo"
  prepare_backlog "$TEST_TMPDIR/repo"
  install_recording_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$ENSURE_LABELS" --repo-root "$TEST_TMPDIR/repo" --confirm

  [ "$status" -eq 0 ]
  [[ "$output" == *"label provisioning: PASS"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-label-apply.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-label-apply.md" ]
  grep -q 'label create autospec:managed' "$TEST_TMPDIR/gh.log"
  grep -q 'label create autospec:follow-up' "$TEST_TMPDIR/gh.log"
}

@test "label provisioning permission failure writes manual instructions" {
  mkdir -p "$TEST_TMPDIR/repo"
  prepare_backlog "$TEST_TMPDIR/repo"
  install_permission_denied_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$ENSURE_LABELS" --repo-root "$TEST_TMPDIR/repo" --confirm

  [ "$status" -eq 1 ]
  [[ "$output" == *"label provisioning: FAIL"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-label-apply.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-label-apply.md" ]
  grep -q 'Manual creation instructions' "$TEST_TMPDIR/repo/.autospec/reports/github-label-apply.md"
  grep -q 'gh label create autospec:managed' "$TEST_TMPDIR/repo/.autospec/reports/github-label-apply.md"
  run jq -r '.manual_instructions[0].label' "$TEST_TMPDIR/repo/.autospec/reports/github-label-apply.json"
  [ "$output" = "autospec:active" ]
}

@test "issue publishing dry-run does not call gh and writes reports" {
  mkdir -p "$TEST_TMPDIR/repo"
  prepare_backlog "$TEST_TMPDIR/repo"
  install_failing_gh "$TEST_TMPDIR/bin"

  PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 0 ]
  [[ "$output" == *"issue publishing: DRY-RUN"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish.md" ]
  [ ! -f "$TEST_TMPDIR/repo/.autospec/state/github-issue-sync-ledger.json" ]
  run jq -r '.side_effects.github_api_calls' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish.json"
  [ "$output" = "false" ]
}

@test "issue publishing confirm creates issues and writes sync ledger" {
  mkdir -p "$TEST_TMPDIR/repo"
  prepare_backlog "$TEST_TMPDIR/repo"
  install_recording_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --confirm

  [ "$status" -eq 0 ]
  [[ "$output" == *"issue publishing: PASS"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/github-issue-sync-ledger.json" ]
  grep -q 'issue create' "$TEST_TMPDIR/gh.log"
  run jq -r '.items["001-test-add-baseline-testing-evidence"].github_number' "$TEST_TMPDIR/repo/.autospec/state/github-issue-sync-ledger.json"
  [ "$output" = "1" ]
}

@test "issue publishing confirm updates ledgered issues instead of duplicating" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/state"
  prepare_backlog "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/repo/.autospec/state/github-issue-sync-ledger.json" <<'JSON'
{
  "version": 1,
  "items": {
    "001-test-add-baseline-testing-evidence": {
      "github_number": 44,
      "github_url": "https://github.com/example/repo/issues/44",
      "body_hash": "old",
      "title": "old"
    }
  }
}
JSON
  install_recording_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --confirm

  [ "$status" -eq 0 ]
  grep -q 'issue edit 44' "$TEST_TMPDIR/gh.log"
  grep -q 'issue create' "$TEST_TMPDIR/gh.log"
  run jq -r '.items["001-test-add-baseline-testing-evidence"].github_number' "$TEST_TMPDIR/repo/.autospec/state/github-issue-sync-ledger.json"
  [ "$output" = "44" ]
}

#!/usr/bin/env bats
# tests/unit/test_github_publishing.bats — guarded GitHub publishing with stubbed gh.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  PLAN="$REPO_ROOT/scripts/autospec-plan-issues.sh"
  BOT_STATE="$REPO_ROOT/scripts/autospec-bot-state-init.sh"
  ENSURE_LABELS="$REPO_ROOT/scripts/autospec-ensure-labels.sh"
  PUBLISH="$REPO_ROOT/scripts/autospec-publish-issues.sh"
  SYNC="$REPO_ROOT/scripts/autospec-sync-published-issues.sh"
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
if [ "$1" = "--repo" ]; then shift 2; fi
if [ "$1" = "issue" ] && [ "$2" = "create" ]; then
  body_file=""
  prev=""
  for arg in "$@"; do
    if [ "$prev" = "--body-file" ]; then body_file="$arg"; fi
    prev="$arg"
  done
  if [ -n "$body_file" ]; then
    cp "$body_file" "${GH_STUB_LOG}.last-body"
    cat "$body_file" >> "${GH_STUB_LOG}.bodies"
  fi
  count_file="${GH_STUB_LOG}.create-count"
  count=0
  [ -f "$count_file" ] && count="$(cat "$count_file")"
  count=$((count + 1))
  printf '%s' "$count" > "$count_file"
  printf 'https://github.com/example/repo/issues/%s\n' "$count"
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
  body_file=""
  prev=""
  for arg in "$@"; do
    if [ "$prev" = "--body-file" ]; then body_file="$arg"; fi
    prev="$arg"
  done
  if [ -n "$body_file" ]; then
    cp "$body_file" "${GH_STUB_LOG}.last-body"
    cat "$body_file" >> "${GH_STUB_LOG}.bodies"
  fi
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

install_label_failing_issue_gh() {
  local bin="$1"
  local log="$2"
  mkdir -p "$bin"
  cat > "$bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_STUB_LOG"
if [ "$1" = "--repo" ]; then shift 2; fi
if [ "$1" = "issue" ] && [ "$2" = "create" ]; then
  for arg in "$@"; do
    if [ "$arg" = "autospec:managed" ]; then
      printf 'label autospec:managed not found\n' >&2
      exit 1
    fi
  done
  printf 'https://github.com/example/repo/issues/9\n'
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
  for arg in "$@"; do
    if [ "$arg" = "autospec:managed" ]; then
      printf 'label autospec:managed not found\n' >&2
      exit 1
    fi
  done
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
exit 0
SH
  chmod +x "$bin/gh"
  : > "$log"
}

install_marker_existing_gh() {
  local bin="$1"
  local log="$2"
  mkdir -p "$bin"
  cat > "$bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_STUB_LOG"
if [ "$1" = "--repo" ]; then shift 2; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  if printf '%s\n' "$*" | grep -q 'autospec-local-issue-id:'; then
    printf '[{"number":77,"url":"https://github.com/example/repo/issues/77","title":"test: add baseline testing evidence","state":"OPEN"}]\n'
  else
    printf '[]\n'
  fi
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
  exit 0
fi
exit 0
SH
  chmod +x "$bin/gh"
  : > "$log"
}

install_title_fallback_gh() {
  local bin="$1"
  local log="$2"
  mkdir -p "$bin"
  cat > "$bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_STUB_LOG"
if [ "$1" = "--repo" ]; then shift 2; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  if printf '%s\n' "$*" | grep -q 'autospec-local-issue-id:'; then
    printf '[]\n'
  else
    printf '[{"number":88,"url":"https://github.com/example/repo/issues/88","title":"test: add baseline testing evidence","state":"OPEN"}]\n'
  fi
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
  exit 0
fi
exit 0
SH
  chmod +x "$bin/gh"
  : > "$log"
}

install_closed_issue_gh() {
  local bin="$1"
  local log="$2"
  mkdir -p "$bin"
  cat > "$bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_STUB_LOG"
if [ "$1" = "--repo" ]; then shift 2; fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  printf '{"number":44,"url":"https://github.com/example/repo/issues/44","title":"old","state":"CLOSED","labels":[{"name":"autospec:managed"}]}\n'
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "reopen" ]; then
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
  exit 0
fi
exit 0
SH
  chmod +x "$bin/gh"
  : > "$log"
}

install_permission_denied_issue_gh() {
  local bin="$1"
  local log="$2"
  mkdir -p "$bin"
  cat > "$bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_STUB_LOG"
if [ "$1" = "--repo" ]; then shift 2; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "create" ]; then
  printf 'HTTP 403: Resource not accessible by integration\n' >&2
  exit 1
fi
exit 0
SH
  chmod +x "$bin/gh"
  : > "$log"
}

install_sync_gh() {
  local bin="$1"
  local log="$2"
  mkdir -p "$bin"
  cat > "$bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_STUB_LOG"
if [ "$1" = "--repo" ]; then shift 2; fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  printf '{"number":44,"url":"https://github.com/example/repo/issues/44","title":"published title","state":"OPEN","labels":[{"name":"autospec:managed"},{"name":"autospec:needs-guidance"}]}\n'
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

  PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --dry-run

  [ "$status" -eq 0 ]
  [[ "$output" == *"issue publishing: DRY-RUN"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-plan.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-plan.md" ]
  [ ! -f "$TEST_TMPDIR/repo/.autospec/state/published-issues.json" ]
  run jq -r '.side_effects.github_api_calls' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-plan.json"
  [ "$output" = "false" ]
}

@test "issue publishing absent config defaults to dry-run without GitHub calls" {
  mkdir -p "$TEST_TMPDIR/repo"
  prepare_backlog "$TEST_TMPDIR/repo"
  rm -f "$TEST_TMPDIR/repo/.autospec/autospec.yml"
  install_failing_gh "$TEST_TMPDIR/bin"

  PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo"

  [ "$status" -eq 0 ]
  [[ "$output" == *"issue publishing: DRY-RUN"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-plan.md" ]
  [ ! -f "$TEST_TMPDIR/repo/.autospec/state/published-issues.json" ]
}

@test "issue publishing confirm creates issues and writes sync ledger" {
  mkdir -p "$TEST_TMPDIR/repo"
  prepare_backlog "$TEST_TMPDIR/repo"
  install_recording_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --confirm

  [ "$status" -eq 0 ]
  [[ "$output" == *"issue publishing: PASS"* ]]
  [ -f "$TEST_TMPDIR/repo/.autospec/state/published-issues.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-result.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-result.md" ]
  grep -q 'issue create' "$TEST_TMPDIR/gh.log"
  grep -q '<!-- autospec-local-issue-id: 001-test-add-baseline-testing-evidence -->' "$TEST_TMPDIR/gh.log.bodies"
  grep -q '<!-- autospec-source-gap-hash:' "$TEST_TMPDIR/gh.log.bodies"
  grep -q '<!-- autospec-body-hash:' "$TEST_TMPDIR/gh.log.bodies"
  run jq -r '.issues[] | select(.local_issue_id=="001-test-add-baseline-testing-evidence") | .github_issue_number' "$TEST_TMPDIR/repo/.autospec/state/published-issues.json"
  [ "$output" = "1" ]
}

@test "issue publishing writes schema 1 issues-array ledger with repo and timestamps" {
  mkdir -p "$TEST_TMPDIR/repo"
  prepare_backlog "$TEST_TMPDIR/repo"
  install_recording_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --repo example/repo --confirm

  [ "$status" -eq 0 ]
  run jq -r '.schema' "$TEST_TMPDIR/repo/.autospec/state/published-issues.json"
  [ "$output" = "1" ]
  run jq -r '.repo' "$TEST_TMPDIR/repo/.autospec/state/published-issues.json"
  [ "$output" = "example/repo" ]
  run jq -r '.issues[0] | [.local_issue_id,.github_issue_number,.state,.last_published_at,.last_synced_at] | @tsv' "$TEST_TMPDIR/repo/.autospec/state/published-issues.json"
  [[ "$output" == 001-test-add-baseline-testing-evidence$'\t'1$'\t'open$'\t'*Z$'\t'*Z ]]
}

@test "issue publishing confirm updates ledgered issues instead of duplicating" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/state"
  prepare_backlog "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/repo/.autospec/state/published-issues.json" <<'JSON'
{
  "schema": 1,
  "repo": "example/repo",
  "issues": [
    {
      "local_issue_id": "001-test-add-baseline-testing-evidence",
      "github_issue_number": 44,
      "github_issue_url": "https://github.com/example/repo/issues/44",
      "body_hash": "old",
      "title": "old",
      "state": "open"
    }
  ]
}
JSON
  install_recording_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --confirm

  [ "$status" -eq 0 ]
  grep -q 'issue edit 44' "$TEST_TMPDIR/gh.log"
  grep -q 'issue create' "$TEST_TMPDIR/gh.log"
  run jq -r '.issues[] | select(.local_issue_id=="001-test-add-baseline-testing-evidence") | .github_issue_number' "$TEST_TMPDIR/repo/.autospec/state/published-issues.json"
  [ "$output" = "44" ]
}

@test "issue publishing confirm still creates issue when labels fail and reports failed labels" {
  mkdir -p "$TEST_TMPDIR/repo"
  prepare_backlog "$TEST_TMPDIR/repo"
  install_label_failing_issue_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --confirm

  [ "$status" -eq 0 ]
  [[ "$output" == *"issue publishing: PASS"* ]]
  grep -q 'issue create --title test: add baseline testing evidence --body-file' "$TEST_TMPDIR/gh.log"
  grep -q 'issue edit 9 --add-label autospec:managed' "$TEST_TMPDIR/gh.log"
  run jq -r '.actions[].label_failures[]?' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-result.json"
  [[ "$output" == *"autospec:managed"* ]]
}

@test "issue publishing links existing open issue by marker before creating duplicate" {
  mkdir -p "$TEST_TMPDIR/repo"
  prepare_backlog "$TEST_TMPDIR/repo"
  install_marker_existing_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --repo example/repo --confirm

  [ "$status" -eq 0 ]
  grep -q 'issue list --search autospec-local-issue-id: 001-test-add-baseline-testing-evidence' "$TEST_TMPDIR/gh.log"
  grep -q 'issue edit 77' "$TEST_TMPDIR/gh.log"
  ! grep -q 'issue create' "$TEST_TMPDIR/gh.log"
  run jq -r '.issues[] | select(.local_issue_id=="001-test-add-baseline-testing-evidence") | .github_issue_number' "$TEST_TMPDIR/repo/.autospec/state/published-issues.json"
  [ "$output" = "77" ]
}

@test "issue publishing links exact title fallback with warning before creating duplicate" {
  mkdir -p "$TEST_TMPDIR/repo"
  prepare_backlog "$TEST_TMPDIR/repo"
  install_title_fallback_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --repo example/repo --confirm

  [ "$status" -eq 0 ]
  ! grep -q 'issue create' "$TEST_TMPDIR/gh.log"
  run jq -r '.warnings[]?' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-result.json"
  [[ "$output" == *"exact title fallback"* ]]
  grep -q 'exact title fallback' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-result.md"
}

@test "issue publishing skips closed ledger issue unless reopen flag is supplied" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/state"
  prepare_backlog "$TEST_TMPDIR/repo"
  cat > "$TEST_TMPDIR/repo/.autospec/state/published-issues.json" <<'JSON'
{
  "schema": 1,
  "repo": "example/repo",
  "issues": [
    {
      "local_issue_id": "001-test-add-baseline-testing-evidence",
      "title": "old",
      "github_issue_number": 44,
      "github_issue_url": "https://github.com/example/repo/issues/44",
      "state": "closed"
    }
  ]
}
JSON
  install_closed_issue_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --repo example/repo --confirm

  [ "$status" -eq 0 ]
  grep -q 'issue view 44' "$TEST_TMPDIR/gh.log"
  ! grep -q 'issue reopen 44' "$TEST_TMPDIR/gh.log"
  grep -q 'closed; skipped' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-result.md"

  : > "$TEST_TMPDIR/gh.log"
  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --repo example/repo --confirm --reopen

  [ "$status" -eq 0 ]
  grep -q 'issue reopen 44' "$TEST_TMPDIR/gh.log"
}

@test "issue publishing permission failure writes actionable report" {
  mkdir -p "$TEST_TMPDIR/repo"
  prepare_backlog "$TEST_TMPDIR/repo"
  install_permission_denied_issue_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$PUBLISH" --repo-root "$TEST_TMPDIR/repo" --repo example/repo --confirm

  [ "$status" -eq 1 ]
  [[ "$output" == *"issue publishing: FAIL"* ]]
  grep -q 'Resource not accessible' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-result.md"
  grep -q 'Check GitHub issue permissions' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-publish-result.md"
}

@test "issue sync updates local ledger state and reports drift labels and closed issues" {
  mkdir -p "$TEST_TMPDIR/repo/.autospec/state" "$TEST_TMPDIR/repo/.autospec/backlog/issues"
  cat > "$TEST_TMPDIR/repo/.autospec/state/published-issues.json" <<'JSON'
{
  "schema": 1,
  "repo": "example/repo",
  "issues": [
    {
      "local_issue_id": "001-test-add-baseline-testing-evidence",
      "title": "local title",
      "body_hash": "local",
      "labels": ["autospec:managed"],
      "github_issue_number": 44,
      "github_issue_url": "https://github.com/example/repo/issues/44",
      "state": "open"
    }
  ]
}
JSON
  install_sync_gh "$TEST_TMPDIR/bin" "$TEST_TMPDIR/gh.log"

  GH_STUB_LOG="$TEST_TMPDIR/gh.log" PATH="$TEST_TMPDIR/bin:$PATH" run bash "$SYNC" --repo-root "$TEST_TMPDIR/repo" --repo example/repo

  [ "$status" -eq 0 ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-issue-sync.json" ]
  [ -f "$TEST_TMPDIR/repo/.autospec/reports/github-issue-sync.md" ]
  run jq -r '.issues[0].title' "$TEST_TMPDIR/repo/.autospec/state/published-issues.json"
  [ "$output" = "published title" ]
  run jq -r '.issues[0].labels | join(",")' "$TEST_TMPDIR/repo/.autospec/state/published-issues.json"
  [ "$output" = "autospec:managed,autospec:needs-guidance" ]
  grep -q 'needs guidance' "$TEST_TMPDIR/repo/.autospec/reports/github-issue-sync.md"
}

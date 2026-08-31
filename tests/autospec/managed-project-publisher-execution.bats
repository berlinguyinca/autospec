#!/usr/bin/env bats

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
PROJECT_HELPER="$REPO_ROOT/skills/autospec-shared/scripts/project-sync-issue.sh"

setup() {
  TMP="$(mktemp -d)"
  mkdir -p "$TMP/bin" "$TMP/repo/.autospec"
  git -C "$TMP/repo" init -q -b main
  git -C "$TMP/repo" config user.email autospec-test@example.invalid
  git -C "$TMP/repo" config user.name autospec-test
  git -C "$TMP/repo" commit --allow-empty -q -m seed
  export EVENTS="$TMP/events"
  export AUTOSPEC_CALLS="$TMP/autospec.calls"
  cp "$PROJECT_HELPER" "$TMP/bin/project-sync-issue.sh"
  cat > "$TMP/bin/autospec" <<'SH'
#!/usr/bin/env bash
printf 'autospec:%s\n' "$*" >> "$EVENTS"
printf '%s\n' "$*" >> "$AUTOSPEC_CALLS"
if [ -n "${AUTOSPEC_SYNC_FAIL:-}" ]; then
  [ "$AUTOSPEC_SYNC_FAIL" = hard ] || echo 'journaled_projection_pending: retry later' >&2
  exit 9
fi
SH
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf 'gh:%s\n' "$*" >> "$EVENTS"
case "$1 $2" in
  "issue list") printf '[]\n' ;;
  "issue create") printf 'https://github.com/acme/widgets/issues/77\n' ;;
  "repo view") printf 'acme/widgets\n' ;;
esac
exit 0
SH
  chmod +x "$TMP/bin/autospec" "$TMP/bin/gh" "$TMP/bin/project-sync-issue.sh"
  export PATH="$TMP/bin:$PATH"
}

teardown() {
  rm -rf "$TMP"
  unset EVENTS AUTOSPEC_CALLS AUTOSPEC_SYNC_FAIL AUTOSPEC_SCRIPTS_DIR
}

assert_create_then_sync_once() {
  [ "$(grep -c '^gh:issue create' "$EVENTS")" -eq 1 ]
  [ "$(grep -c '^autospec:project sync' "$EVENTS")" -eq 1 ]
  create_line="$(grep -n '^gh:issue create' "$EVENTS" | cut -d: -f1)"
  sync_line="$(grep -n '^autospec:project sync' "$EVENTS" | cut -d: -f1)"
  [ "$create_line" -lt "$sync_line" ]
}

assert_no_sync() {
  [ ! -e "$AUTOSPEC_CALLS" ] || [ ! -s "$AUTOSPEC_CALLS" ]
}

@test "explore fallback creates before sync and sync failure does not recreate" {
  mkdir -p "$TMP/iter"
  printf '%s\n' '{"proposals":[{"title":"feat: add retry","source":"spec-vs-code","estimated_complexity":"small","confidence":0.9}]}' > "$TMP/proposals.json"
  cat > "$TMP/driver.sh" <<DRIVER
set -u
SCRIPT_DIR="$REPO_ROOT/scripts"
REPO_ROOT="$TMP/repo"
cd "$TMP/repo"
SANDBOX_BRANCH="sandbox/explore-demo"
RESEARCH_SOURCES="spec-vs-code"
GITHUB_REPOSITORY="acme/widgets"
iter=1
research_json="$TMP/proposals.json"
iter_dir="$TMP/iter"
proposals_count=1
issues_filed=0
filed_issue_nums=""
_ledger_append() { :; }
_ledger_normalize_title() { printf '%s' "\$1"; }
_explore_review_exact_issue() { return 0; }
LEDGER_BIN=""
$(awk '/^project_sync_issue\(\)/,/^}/' "$REPO_ROOT/scripts/autospec-explore.sh")
$(awk '/^# >>> explore-spec-first-filing >>>/,/^# <<< explore-spec-first-filing <<</' "$REPO_ROOT/scripts/autospec-explore.sh")
_explore_raw_file_round
DRIVER
  run env AUTOSPEC_SYNC_FAIL=1 AUTOSPEC_SCRIPTS_DIR="$TMP/bin" bash "$TMP/driver.sh"
  [ "$status" -eq 0 ]
  assert_create_then_sync_once

  run env AUTOSPEC_SYNC_FAIL=hard AUTOSPEC_SCRIPTS_DIR="$TMP/bin" bash "$TMP/driver.sh"
  [ "$status" -ne 0 ]

  sed '$d' "$TMP/driver.sh" > "$TMP/wrapped-driver.sh"
  printf '%s\n' '_explore_file_round' >> "$TMP/wrapped-driver.sh"
  run env AUTOSPEC_SYNC_FAIL=hard AUTOSPEC_SCRIPTS_DIR="$TMP/bin" bash "$TMP/wrapped-driver.sh"
  [ "$status" -ne 0 ]

  : > "$EVENTS"; : > "$AUTOSPEC_CALLS"
  run env AUTOSPEC_SCRIPTS_DIR="$TMP/bin" AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD='printf "{\"proposals_total\":0,\"proposals\":[]}"' \
    bash "$REPO_ROOT/scripts/autospec-explore.sh" --once --preview --autonomous --research-sources spec-vs-code
  [ "$status" -eq 0 ]
  run ! grep -q '^gh:issue create' "$EVENTS"
  [ "$status" -ne 0 ]
  assert_no_sync
}

@test "self-improvement apply creates before sync and report-only skips sync" {
  mkdir -p "$TMP/repo/crates/autospec-cli/src/commands"
  printf '%s\n' 'pub fn run() { not_implemented("run"); }' > "$TMP/repo/crates/autospec-cli/src/commands/run.rs"
  run env AUTOSPEC_SYNC_FAIL=1 AUTOSPEC_SCRIPTS_DIR="$TMP/bin" AUTOSPEC_SELF_IMPROVEMENT_APPLY=1 \
    bash "$REPO_ROOT/scripts/autonomous-self-improvement.sh" apply --repo-root "$TMP/repo" --repo acme/widgets --apply --limit 1
  [ "$status" -eq 0 ]
  assert_create_then_sync_once

  : > "$EVENTS"; : > "$AUTOSPEC_CALLS"
  run env AUTOSPEC_SCRIPTS_DIR="$TMP/bin" bash "$REPO_ROOT/scripts/autonomous-self-improvement.sh" \
    apply --repo-root "$TMP/repo" --repo acme/widgets --apply --limit 1
  [ "$status" -eq 0 ]
  run ! grep -q '^gh:issue create' "$EVENTS"
  [ "$status" -ne 0 ]
  assert_no_sync
}

@test "gap miner creates before sync and dry-run skips sync" {
  printf '%s\n' 'REQUEST_CHANGES: add the missing retry test' > "$TMP/gaps.log"
  run env AUTOSPEC_SYNC_FAIL=1 AUTOSPEC_SCRIPTS_DIR="$TMP/bin" bash "$REPO_ROOT/scripts/autospec-gap-miner.sh" \
    --input "$TMP/gaps.log" --ledger "$TMP/ledger.md" --repo acme/widgets --file
  [ "$status" -eq 0 ]
  assert_create_then_sync_once

  : > "$EVENTS"; : > "$AUTOSPEC_CALLS"
  run env AUTOSPEC_SCRIPTS_DIR="$TMP/bin" bash "$REPO_ROOT/scripts/autospec-gap-miner.sh" \
    --input "$TMP/gaps.log" --ledger "$TMP/dry-ledger.md" --repo acme/widgets --dry-run
  [ "$status" -eq 0 ]
  run ! grep -q '^gh:issue create' "$EVENTS"
  [ "$status" -ne 0 ]
  assert_no_sync
}

@test "self issue creates before sync and dry-run skips sync" {
  script="$REPO_ROOT/skills/autospec-shared/scripts/autospec-self-issue.sh"
  finding='{"category":"runtime","summary":"retry failed","evidence":"log"}'
  run env AUTOSPEC_SYNC_FAIL=1 bash "$script" --finding "$finding" --repo acme/widgets --cache "$TMP/live.cache"
  [ "$status" -eq 0 ]
  assert_create_then_sync_once

  run env AUTOSPEC_SYNC_FAIL=hard bash "$script" --finding "$finding" --repo acme/widgets --cache "$TMP/hard.cache"
  [ "$status" -ne 0 ]
  [ ! -e "$TMP/hard.cache" ]

  : > "$EVENTS"; : > "$AUTOSPEC_CALLS"
  run bash "$script" --finding "$finding" --repo acme/widgets --cache "$TMP/dry.cache" --dry-run
  [ "$status" -eq 0 ]
  run ! grep -q '^gh:issue create' "$EVENTS"
  [ "$status" -ne 0 ]
  assert_no_sync
}

@test "doc freshness creates before sync and dry-run skips sync" {
  script="$REPO_ROOT/skills/autospec-shared/scripts/doc-freshness-tier.sh"
  mkdir -p "$TMP/repo/docs" "$TMP/repo/.git"
  cat > "$TMP/check-doc-drift.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' '{"passed":false,"drift":[{"doc_file":"docs/api.md","heading":"Flags","matching_source_files":["scripts/app.sh"],"reason":"changed"}],"missing_scope":[],"visual_stale":[],"example_stale":[],"skipped":false}'
exit 1
SH
  chmod +x "$TMP/check-doc-drift.sh"
  run env AUTOSPEC_SYNC_FAIL=1 AUTOSPEC_CHECK_DRIFT_SH="$TMP/check-doc-drift.sh" bash "$script" \
    --working-tree --repo-root "$TMP/repo"
  [ "$status" -eq 1 ]
  assert_create_then_sync_once

  : > "$EVENTS"; : > "$AUTOSPEC_CALLS"
  run env AUTOSPEC_CHECK_DRIFT_SH="$TMP/check-doc-drift.sh" bash "$script" \
    --working-tree --repo-root "$TMP/repo" --dry-run
  [ "$status" -eq 1 ]
  run ! grep -q '^gh:issue create' "$EVENTS"
  [ "$status" -ne 0 ]
  assert_no_sync
}

@test "gap remediation creates before sync and no-file mode skips sync" {
  script="$REPO_ROOT/skills/autospec-shared/scripts/gap-remediation-loop.sh"
  fixture="$REPO_ROOT/skills/autospec-shared/tests/fixtures/gap-valid.json"
  run env AUTOSPEC_SYNC_FAIL=1 AUTOSPEC_STATE_DIR="$TMP/state-live" AUTOSPEC_GAP_REPO=acme/widgets \
    bash "$script" --gaps "$fixture" --file
  [ "$status" -eq 0 ]
  assert_create_then_sync_once

  : > "$EVENTS"; : > "$AUTOSPEC_CALLS"
  run env AUTOSPEC_STATE_DIR="$TMP/state-dry" AUTOSPEC_GAP_REPO=acme/widgets bash "$script" --gaps "$fixture"
  [ "$status" -eq 0 ]
  run ! grep -q '^gh:issue create' "$EVENTS"
  [ "$status" -ne 0 ]
  assert_no_sync
}

@test "grow define creates before sync and sync failure does not recreate" {
  script="$REPO_ROOT/skills/autospec-shared/scripts/grow-define-file-issues.sh"
  printf '%s\n' '{"lens":"keyword-gap","channel":"content","kind":"artifact","title":"Add comparison","norm_title":"add comparison","rationale":"missing page"}' > "$TMP/ranked.jsonl"
  printf '%s\n' '{"product":{"name":"Acme"},"site":{"repo_path":"."}}' > "$TMP/growth.json"
  run env AUTOSPEC_SYNC_FAIL=1 GROWTH_LEDGER="$TMP/growth-ledger.jsonl" bash "$script" "$TMP/ranked.jsonl" "$TMP/growth.json"
  [ "$status" -eq 0 ]
  assert_create_then_sync_once
}

@test "quality audit mutates each issue before one sync and read-only mode skips sync" {
  script="$REPO_ROOT/skills/autospec-shared/scripts/repo-quality-audit.sh"
  mkdir -p "$TMP/audit-repo/src"
  printf '%s\n' 'console.log("debug");' > "$TMP/audit-repo/src/main.js"
  run env AUTOSPEC_SYNC_FAIL=1 AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES=1 \
    AUTOSPEC_QUALITY_AUDIT_DEBUG_THRESHOLD=1 bash "$script" --repo "$TMP/audit-repo" \
    --json "$TMP/audit.json" --markdown "$TMP/audit.md" --file-issues
  [ "$status" -eq 0 ]
  creates="$(grep -c '^gh:issue create' "$EVENTS")"
  syncs="$(grep -c '^autospec:project sync' "$EVENTS")"
  [ "$creates" -gt 0 ]
  [ "$creates" -eq "$syncs" ]
  awk '
    /^gh:issue create/ { if (pending) exit 1; pending=1; next }
    /^autospec:project sync/ { if (!pending) exit 1; pending=0 }
    END { exit pending ? 1 : 0 }
  ' "$EVENTS"

  : > "$EVENTS"; : > "$AUTOSPEC_CALLS"
  run bash "$script" --repo "$TMP/audit-repo" --json "$TMP/read-only.json" --markdown "$TMP/read-only.md"
  [ "$status" -eq 0 ]
  run ! grep -q '^gh:issue create' "$EVENTS"
  [ "$status" -ne 0 ]
  assert_no_sync
}

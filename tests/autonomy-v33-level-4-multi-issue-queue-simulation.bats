#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
RUN_ID="autonomy-v33-level-4-autospec"

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/tests" "$TEST_TMP/repo/tests"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs" 2>/dev/null || mkdir -p "$TEST_TMP/repo/docs"
  mkdir -p "$TEST_TMP/repo/.autospec/reports"
  printf '# Fixture\n' > "$TEST_TMP/repo/README.md"
  git -C "$TEST_TMP/repo" init -q
  git -C "$TEST_TMP/repo" config user.email test@example.com
  git -C "$TEST_TMP/repo" config user.name Test
  git -C "$TEST_TMP/repo" add README.md scripts tests docs >/dev/null 2>&1
  git -C "$TEST_TMP/repo" commit -qm init >/dev/null 2>&1 || true
  git -C "$TEST_TMP/repo" checkout -b autospec/v33-queue-simulation-test >/dev/null 2>&1
  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v31-human-approved-draft-pr-update-commit-and-.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v32-human-approved-pr-conversation-response-pa.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v33-draft-pr-update-transaction-harness-and-re.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v34-level-4-issue-publishing-canary.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v35-single-issue-to-draft-pr-real-loop-canary.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v36-issue-to-pr-recovery-duplicate-and-idempot.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v37-backlog-triage-and-prioritization-governan.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "v33 simulation supervisor writes finite mock queue artifacts and zero-write audit" {
  run bash "$TEST_TMP/repo/scripts/autospec-supervisor-v38-level-4-multi-issue-queue-simulation.sh" --repo-root "$TEST_TMP/repo" --prepare-only
  [ "$status" -eq 0 ]
  for file in contract preflight artifact-index gate audit verifier recovery v33-status; do
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v33/$RUN_ID/$file.json" ]
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v33/$RUN_ID/$file.md" ]
  done
  [ -f "$TEST_TMP/repo/.autospec/autonomy/v33/$RUN_ID/closeout.md" ]
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v33-status.json" <<'PY'
import json, sys
s=json.load(open(sys.argv[1]))
assert s["status"] == "ready"
assert s["previous_statuses"] == "ready"
assert s["phase_goal_satisfied"] is True
assert s["finite_queue_verified"] is True
assert s["mock_issues_used"] is True
assert s["mock_prs_used"] is True
assert s["lease_expiry_verified"] is True
assert s["stop_decisions_verified"] is True
assert s["duplicate_prevention_verified"] is True
assert s["unbounded_loop_prevented"] is True
for key in ["network_attempted","github_write_attempted","git_push_attempted","draft_pr_create_attempted","pr_update_attempted","issue_publishing_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"]:
    assert s[key] is False, key
PY
}

@test "v33 blocks missing prior-version evidence" {
  rm -f "$TEST_TMP/repo/.autospec/reports/autonomy-v32-status.json"
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v33-gate.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "blocked_missing_prior_evidence" "$TEST_TMP/repo/.autospec/autonomy/v33/$RUN_ID/gate.json"
}

@test "v33 blocks unsafe default branch" {
  git -C "$TEST_TMP/repo" checkout -B main >/dev/null 2>&1
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v33-preflight.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "blocked_unsafe_branch" "$TEST_TMP/repo/.autospec/autonomy/v33/$RUN_ID/preflight.json"
}

@test "v33 blocks real writes and network-looking flags" {
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v33-gate.sh" --repo-root "$TEST_TMP/repo" --confirm --allow-network --allow-git-push --execute-real-github-write
  [ "$status" -ne 0 ]
  grep -q "blocked_forbidden_operation:local_mock_only" "$TEST_TMP/repo/.autospec/autonomy/v33/$RUN_ID/gate.json"
}

@test "v33 audit is canonical local/mock negative proof" {
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v33-audit.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  python3 - "$TEST_TMP/repo/.autospec/autonomy/v33/$RUN_ID/audit.json" <<'PY'
import json, sys
a=json.load(open(sys.argv[1]))
assert a["phase"] == "v33"
assert a["mode"] == "simulation_only"
assert a["finite_queue_verified"] is True
assert a["mock_issues_used"] is True
assert a["mock_prs_used"] is True
for key in ["network_attempted","github_read_attempted","github_write_attempted","git_push_attempted","draft_pr_create_attempted","pr_update_attempted","issue_publishing_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"]:
    assert a[key] is False, key
assert a["scheduler"] == "absent"
assert a["daemon"] == "absent"
assert a["background_runner"] == "absent"
PY
}

@test "v33 recovery is foreground-only and never auto-resumes" {
  bash "$TEST_TMP/repo/scripts/autospec-autonomous-v33-audit.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-autonomous-v33-verifier.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v33-recovery.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  grep -q '"auto_resume": false' "$TEST_TMP/repo/.autospec/autonomy/v33/$RUN_ID/recovery.json"
  grep -q '"foreground_only": true' "$TEST_TMP/repo/.autospec/autonomy/v33/$RUN_ID/recovery.json"
}

@test "v33 status refuses ready if audit artifact is missing" {
  bash "$TEST_TMP/repo/scripts/autospec-autonomous-v33-contract.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  rm -f "$TEST_TMP/repo/.autospec/autonomy/v33/$RUN_ID/audit.json" "$TEST_TMP/repo/.autospec/reports/autonomous-v33-audit.json"
  run bash "$TEST_TMP/repo/scripts/autospec-v33-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "missing_audit_artifact" "$TEST_TMP/repo/.autospec/autonomy/v33/$RUN_ID/v33-status.json"
}

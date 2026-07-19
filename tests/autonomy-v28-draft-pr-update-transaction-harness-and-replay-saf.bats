#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
RUN_ID="autonomy-v28-level-3-autospec"

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
  git -C "$TEST_TMP/repo" checkout -b autospec/v28-transaction-simulation-test >/dev/null 2>&1
  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v31-human-approved-draft-pr-update-commit-and-.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v32-human-approved-pr-conversation-response-pa.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "v28 simulation supervisor writes required artifacts and zero-write audit" {
  run bash "$TEST_TMP/repo/scripts/autospec-supervisor-v33-draft-pr-update-transaction-harness-and-re.sh" --repo-root "$TEST_TMP/repo" --prepare-only
  [ "$status" -eq 0 ]
  for file in contract preflight artifact-index gate audit verifier recovery v28-status; do
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v28/$RUN_ID/$file.json" ]
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v28/$RUN_ID/$file.md" ]
  done
  [ -f "$TEST_TMP/repo/.autospec/autonomy/v28/$RUN_ID/closeout.md" ]
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v28-status.json" <<'PY'
import json, sys
s=json.load(open(sys.argv[1]))
assert s["status"] == "ready"
assert s["previous_statuses"] == "ready"
assert s["phase_goal_satisfied"] is True
assert s["idempotency_keys_written"] is True
assert s["duplicate_update_prevention_verified"] is True
for key in ["network_attempted","github_write_attempted","git_push_attempted","pr_update_attempted","issue_publishing_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"]:
    assert s[key] is False, key
PY
}

@test "v28 blocks missing prior-version evidence" {
  rm -f "$TEST_TMP/repo/.autospec/reports/autonomy-v27-status.json"
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v28-gate.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "blocked_missing_prior_evidence" "$TEST_TMP/repo/.autospec/autonomy/v28/$RUN_ID/gate.json"
}

@test "v28 blocks unsafe default branch" {
  git -C "$TEST_TMP/repo" checkout -B main >/dev/null 2>&1
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v28-preflight.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "blocked_unsafe_branch" "$TEST_TMP/repo/.autospec/autonomy/v28/$RUN_ID/preflight.json"
}

@test "v28 blocks network and real GitHub writes even with write-looking flags" {
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v28-gate.sh" --repo-root "$TEST_TMP/repo" --confirm --allow-network --allow-git-push --execute-real-github-write
  [ "$status" -ne 0 ]
  grep -q "blocked_forbidden_operation:real_writes_blocked_in_v28" "$TEST_TMP/repo/.autospec/autonomy/v28/$RUN_ID/gate.json"
}

@test "v28 audit is canonical simulation-only negative proof" {
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v28-audit.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  python3 - "$TEST_TMP/repo/.autospec/autonomy/v28/$RUN_ID/audit.json" <<'PY'
import json, sys
a=json.load(open(sys.argv[1]))
assert a["phase"] == "v28"
assert a["mode"] == "simulation_only"
assert a["local_mock_replay_attempted"] is True
assert a["idempotency_keys_written"] is True
for key in ["network_attempted","github_read_attempted","github_write_attempted","git_push_attempted","pr_update_attempted","issue_publishing_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"]:
    assert a[key] is False, key
assert a["scheduler"] == "absent"
assert a["daemon"] == "absent"
assert a["background_runner"] == "absent"
PY
}

@test "v28 recovery is foreground-only and never auto-resumes" {
  bash "$TEST_TMP/repo/scripts/autospec-autonomous-v28-audit.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-autonomous-v28-verifier.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v28-recovery.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  grep -q '"auto_resume": false' "$TEST_TMP/repo/.autospec/autonomy/v28/$RUN_ID/recovery.json"
  grep -q '"foreground_only": true' "$TEST_TMP/repo/.autospec/autonomy/v28/$RUN_ID/recovery.json"
}

@test "v28 status refuses ready if audit artifact is missing" {
  bash "$TEST_TMP/repo/scripts/autospec-autonomous-v28-contract.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  rm -f "$TEST_TMP/repo/.autospec/autonomy/v28/$RUN_ID/audit.json" "$TEST_TMP/repo/.autospec/reports/autonomous-v28-audit.json"
  run bash "$TEST_TMP/repo/scripts/autospec-v28-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "missing_audit_artifact" "$TEST_TMP/repo/.autospec/autonomy/v28/$RUN_ID/v28-status.json"
}

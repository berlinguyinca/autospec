#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
RUN_ID="autonomy-v26-level-3-autospec"

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
  git -C "$TEST_TMP/repo" checkout -b autospec/v26-update-canary-test >/dev/null 2>&1
  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "v26 prepare-only supervisor writes required artifacts and zero-write audit" {
  run bash "$TEST_TMP/repo/scripts/autospec-supervisor-v31-human-approved-draft-pr-update-commit-and-.sh" --repo-root "$TEST_TMP/repo" --prepare-only
  [ "$status" -eq 0 ]
  for file in contract preflight artifact-index gate audit verifier recovery v26-status; do
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v26/$RUN_ID/$file.json" ]
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v26/$RUN_ID/$file.md" ]
  done
  [ -f "$TEST_TMP/repo/.autospec/autonomy/v26/$RUN_ID/closeout.md" ]
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v26-status.json" <<'PY'
import json, sys
s=json.load(open(sys.argv[1]))
assert s["status"] == "ready_after_human_canary"
assert s["previous_statuses"] == "ready"
assert s["phase_goal_satisfied"] is True
for key in ["network_attempted","github_write_attempted","git_push_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"]:
    assert s[key] is False, key
PY
}

@test "v26 blocks missing prior-version evidence" {
  rm -f "$TEST_TMP/repo/.autospec/reports/autonomy-v25-status.json"
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v26-gate.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "blocked_missing_prior_evidence" "$TEST_TMP/repo/.autospec/autonomy/v26/$RUN_ID/gate.json"
}

@test "v26 blocks unsafe default branch" {
  git -C "$TEST_TMP/repo" checkout -B main >/dev/null 2>&1
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v26-preflight.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "blocked_unsafe_branch" "$TEST_TMP/repo/.autospec/autonomy/v26/$RUN_ID/preflight.json"
}

@test "v26 real canary remains blocked without verified approval capsule" {
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v26-gate.sh" --repo-root "$TEST_TMP/repo" --confirm --allow-network --allow-git-push --execute-real-github-write
  [ "$status" -ne 0 ]
  grep -q "blocked_missing_approval_capsule" "$TEST_TMP/repo/.autospec/autonomy/v26/$RUN_ID/gate.json"
}

@test "v26 audit is canonical negative proof" {
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v26-audit.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  python3 - "$TEST_TMP/repo/.autospec/autonomy/v26/$RUN_ID/audit.json" <<'PY'
import json, sys
a=json.load(open(sys.argv[1]))
assert a["phase"] == "v26"
assert a["mode"] == "single_update_canary"
for key in ["network_attempted","github_read_attempted","github_write_attempted","git_push_attempted","pr_update_attempted","issue_publishing_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"]:
    assert a[key] is False, key
assert a["scheduler"] == "absent"
assert a["daemon"] == "absent"
assert a["background_runner"] == "absent"
PY
}

@test "v26 status refuses ready if audit artifact is missing" {
  bash "$TEST_TMP/repo/scripts/autospec-autonomous-v26-contract.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  rm -f "$TEST_TMP/repo/.autospec/autonomy/v26/$RUN_ID/audit.json" "$TEST_TMP/repo/.autospec/reports/autonomous-v26-audit.json"
  run bash "$TEST_TMP/repo/scripts/autospec-v26-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "missing_audit_artifact" "$TEST_TMP/repo/.autospec/autonomy/v26/$RUN_ID/v26-status.json"
}

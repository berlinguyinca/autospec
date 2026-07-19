#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs"
  for v in 61 62 63 64 65 66 67 68; do bash "$TEST_TMP/repo/scripts/autospec-v${v}-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null; done
}

teardown() { rm -rf "$TEST_TMP"; }

@test "v69 prepare-only canary readiness writes approval and recovery artifacts" {
  for script in external-canary-remote-bind external-canary-readiness external-approval-template external-approval-verify external-arm-gate external-push external-draft-pr-create external-pr-verifier external-remote-write-audit external-recovery-plan; do
    bash "$TEST_TMP/repo/scripts/autospec-v69-$script.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  done
  run bash "$TEST_TMP/repo/scripts/autospec-v69-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"ready_for_human_canary"* ]]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v69/canary-readiness.md" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v69/approval-capsule-template.json" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v69/remote-write-audit.md" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v69/recovery-plan.md" ]
}

@test "v69 refuses real write flags without approved capsule" {
  run bash "$TEST_TMP/repo/scripts/autospec-v69-status.sh" --repo-root "$TEST_TMP/repo" --allow-network --allow-git-push --allow-github-pr --execute-real-github-write
  [ "$status" -ne 0 ]
  [[ "$output" == *"blocked"* ]]
  bash "$TEST_TMP/repo/scripts/autospec-v69-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v69-status.json" <<'PY'
import json, sys
s=json.load(open(sys.argv[1]))
assert s["status"] == "ready_for_human_canary"
assert s["real_execution_blocked_without_approval_capsule"] is True
assert s["draft_pr_create_attempted"] is False
assert s["merge_attempted"] is False
assert s["self_approval_attempted"] is False
PY
}

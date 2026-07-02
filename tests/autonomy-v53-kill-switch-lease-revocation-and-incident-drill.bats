#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
RUN_ID="autonomy-v53-incident-drill"

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
  git -C "$TEST_TMP/repo" checkout -b autospec/v53-incident-drill-test >/dev/null 2>&1
  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v31-human-approved-draft-pr-update-commit-and-.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v32-human-approved-pr-conversation-response-pa.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v33-draft-pr-update-transaction-harness-and-re.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v34-level-4-issue-publishing-canary.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v35-single-issue-to-draft-pr-real-loop-canary.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v36-issue-to-pr-recovery-duplicate-and-idempot.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v37-backlog-triage-and-prioritization-governan.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v38-level-4-multi-issue-queue-simulation.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v39-human-approved-level-4-multi-issue-canary.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v40-review-driven-low-risk-source-patch-planni.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v41-controlled-low-risk-source-disposable-patc.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v42-low-risk-source-local-commit-canary.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v43-low-risk-source-draft-pr-canary.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v44-ci-failure-read-only-diagnostics-and-patch.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v45-ci-failure-local-fix-simulation.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v46-dependency-update-planning-and-lockfile-sa.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v47-single-dependency-update-disposable-proof.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v48-single-dependency-update-draft-pr-canary.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v49-security-and-privacy-finding-triage-read-o.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v50-security-and-privacy-patch-planning-gate.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v51-security-and-privacy-disposable-patch-proo.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v52-companion-repo-governance-proposal-pr-cana.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v53-constitution-baseline-drift-reconciliation.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v54-cross-repo-learning-evaluation-harness.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v55-control-plane-observability-and-operator-d.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v56-visible-foreground-queue-service-readiness.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-supervisor-v57-operator-attended-queue-runner-canary.sh" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "v53 supervisor writes local incident drill artifacts" {
  run bash "$TEST_TMP/repo/scripts/autospec-supervisor-v58-kill-switch-lease-revocation-and-incident-.sh" --repo-root "$TEST_TMP/repo" --prepare-only
  [ "$status" -eq 0 ]
  for file in contract preflight artifact-index gate audit verifier recovery v53-status; do
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/$file.json" ]
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/$file.md" ]
  done
  for file in kill-switch-drill lease-revocation-drill stale-lock-drill partial-transaction-drill audit-trail-drill failed-safe-handoff; do
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/$file.json" ]
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/$file.md" ]
  done
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v53-status.json" <<'PY'
import json, sys
s=json.load(open(sys.argv[1]))
assert s["status"] == "ready"
assert s["previous_statuses"] == "ready"
assert s["phase_goal_satisfied"] is True
assert s["mode"] == "drill"
assert s["kill_switch_drill_written"] is True
assert s["lease_revocation_drill_written"] is True
assert s["stale_lock_drill_written"] is True
assert s["partial_transaction_drill_written"] is True
assert s["audit_trail_drill_written"] is True
assert s["failed_safe_handoff_written"] is True
assert s["incident_actions_executed"] == 0
for key in ["network_attempted","github_write_attempted","git_push_attempted","pr_update_attempted","issue_publishing_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"]:
    assert s[key] is False, key
PY
}

@test "v53 blocks missing v52 evidence" {
  rm -f "$TEST_TMP/repo/.autospec/reports/autonomy-v52-status.json"
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v53-gate.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "blocked_missing_prior_evidence" "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/gate.json"
}

@test "v53 blocks unsafe default branch" {
  git -C "$TEST_TMP/repo" checkout -B main >/dev/null 2>&1
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v53-preflight.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "blocked_unsafe_branch" "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/preflight.json"
}

@test "v53 blocks network GitHub writes merge approval default force and tag requests" {
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v53-gate.sh" --repo-root "$TEST_TMP/repo" --confirm --allow-network --allow-git-push --allow-github-pr --allow-merge --allow-auto-merge --allow-approval --allow-self-approval --allow-default-branch-push --allow-force-push --allow-tag-push
  [ "$status" -ne 0 ]
  grep -q "blocked_forbidden_operation:network_not_allowed" "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/gate.json"
  grep -q "blocked_forbidden_operation:github_write_requested" "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/gate.json"
  grep -q "blocked_forbidden_operation:merge_requested" "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/gate.json"
  grep -q "blocked_forbidden_operation:approval_requested" "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/gate.json"
  grep -q "blocked_forbidden_operation:default_branch_push_requested" "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/gate.json"
  grep -q "blocked_forbidden_operation:force_push_requested" "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/gate.json"
  grep -q "blocked_forbidden_operation:tag_push_requested" "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/gate.json"
}

@test "v53 audit proves drill had no real incident side effects" {
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v53-audit.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  python3 - "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/audit.json" <<'PY'
import json, sys
a=json.load(open(sys.argv[1]))
assert a["phase"] == "v53"
assert a["mode"] == "drill"
for key in ["network_attempted","github_write_attempted","git_push_attempted","pr_update_attempted","issue_publishing_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"]:
    assert a[key] is False, key
assert a["scheduler"] == "absent"
assert a["daemon"] == "absent"
assert a["background_runner"] == "absent"
PY
}

@test "v53 status refuses ready if audit artifact is missing" {
  bash "$TEST_TMP/repo/scripts/autospec-autonomous-v53-contract.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  rm -f "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/audit.json" "$TEST_TMP/repo/.autospec/reports/autonomous-v53-audit.json"
  run bash "$TEST_TMP/repo/scripts/autospec-v53-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "missing_audit_artifact" "$TEST_TMP/repo/.autospec/autonomy/v53/$RUN_ID/v53-status.json"
}

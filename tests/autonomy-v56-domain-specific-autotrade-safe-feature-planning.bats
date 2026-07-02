#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
RUN_ID="autonomy-v56-autotrade-feature-planning"

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
  git -C "$TEST_TMP/repo" checkout -b autospec/v56-autotrade-planning-test >/dev/null 2>&1
  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  for script in \
    autospec-supervisor-v31-human-approved-draft-pr-update-commit-and-.sh \
    autospec-supervisor-v32-human-approved-pr-conversation-response-pa.sh \
    autospec-supervisor-v33-draft-pr-update-transaction-harness-and-re.sh \
    autospec-supervisor-v34-level-4-issue-publishing-canary.sh \
    autospec-supervisor-v35-single-issue-to-draft-pr-real-loop-canary.sh \
    autospec-supervisor-v36-issue-to-pr-recovery-duplicate-and-idempot.sh \
    autospec-supervisor-v37-backlog-triage-and-prioritization-governan.sh \
    autospec-supervisor-v38-level-4-multi-issue-queue-simulation.sh \
    autospec-supervisor-v39-human-approved-level-4-multi-issue-canary.sh \
    autospec-supervisor-v40-review-driven-low-risk-source-patch-planni.sh \
    autospec-supervisor-v41-controlled-low-risk-source-disposable-patc.sh \
    autospec-supervisor-v42-low-risk-source-local-commit-canary.sh \
    autospec-supervisor-v43-low-risk-source-draft-pr-canary.sh \
    autospec-supervisor-v44-ci-failure-read-only-diagnostics-and-patch.sh \
    autospec-supervisor-v45-ci-failure-local-fix-simulation.sh \
    autospec-supervisor-v46-dependency-update-planning-and-lockfile-sa.sh \
    autospec-supervisor-v47-single-dependency-update-disposable-proof.sh \
    autospec-supervisor-v48-single-dependency-update-draft-pr-canary.sh \
    autospec-supervisor-v49-security-and-privacy-finding-triage-read-o.sh \
    autospec-supervisor-v50-security-and-privacy-patch-planning-gate.sh \
    autospec-supervisor-v51-security-and-privacy-disposable-patch-proo.sh \
    autospec-supervisor-v52-companion-repo-governance-proposal-pr-cana.sh \
    autospec-supervisor-v53-constitution-baseline-drift-reconciliation.sh \
    autospec-supervisor-v54-cross-repo-learning-evaluation-harness.sh \
    autospec-supervisor-v55-control-plane-observability-and-operator-d.sh \
    autospec-supervisor-v56-visible-foreground-queue-service-readiness.sh \
    autospec-supervisor-v57-operator-attended-queue-runner-canary.sh \
    autospec-supervisor-v58-kill-switch-lease-revocation-and-incident-.sh \
    autospec-supervisor-v59-multi-repo-portfolio-read-only-planning.sh \
    autospec-supervisor-v60-multi-repo-disposable-change-simulation.sh; do
    bash "$TEST_TMP/repo/scripts/$script" --repo-root "$TEST_TMP/repo" --prepare-only >/dev/null
  done
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "v56 supervisor writes Autotrade planning artifacts and blocks high-risk domains" {
  run bash "$TEST_TMP/repo/scripts/autospec-supervisor-v61-domain-specific-autotrade-safe-feature-pla.sh" --repo-root "$TEST_TMP/repo" --prepare-only
  [ "$status" -eq 0 ]
  for file in contract preflight artifact-index gate audit verifier recovery v56-status; do
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/$file.json" ]
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/$file.md" ]
  done
  for file in autotrade-feature-plan domain-safety-boundaries blocked-categories-report candidate-feature-ranking; do
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/$file.json" ]
    [ -f "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/$file.md" ]
  done
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v56-status.json" <<'PY'
import json, sys
s=json.load(open(sys.argv[1]))
assert s["status"] == "ready"
assert s["previous_statuses"] == "ready"
assert s["phase_goal_satisfied"] is True
assert s["mode"] == "planning_only"
assert s["autotrade_feature_plan_written"] is True
assert s["domain_safety_boundaries_written"] is True
assert s["blocked_categories_report_written"] is True
assert s["candidate_feature_ranking_written"] is True
assert s["implementation_attempted"] is False
assert s["trading_execution_changes_attempted"] is False
assert s["secret_changes_attempted"] is False
assert s["migration_changes_attempted"] is False
assert s["auth_changes_attempted"] is False
assert s["deployment_changes_attempted"] is False
for key in ["network_attempted","github_write_attempted","git_push_attempted","pr_update_attempted","issue_publishing_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"]:
    assert s[key] is False, key
PY
}

@test "v56 blocks missing v55 evidence" {
  rm -f "$TEST_TMP/repo/.autospec/reports/autonomy-v55-status.json"
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v56-gate.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "blocked_missing_prior_evidence" "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/gate.json"
}

@test "v56 blocks unsafe default branch" {
  git -C "$TEST_TMP/repo" checkout -B main >/dev/null 2>&1
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v56-preflight.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "blocked_unsafe_branch" "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/preflight.json"
}

@test "v56 blocks network GitHub writes merge approval default force and tag requests" {
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v56-gate.sh" --repo-root "$TEST_TMP/repo" --confirm --allow-network --allow-git-push --allow-github-pr --allow-merge --allow-auto-merge --allow-approval --allow-self-approval --allow-default-branch-push --allow-force-push --allow-tag-push
  [ "$status" -ne 0 ]
  grep -q "blocked_forbidden_operation:network_not_allowed" "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/gate.json"
  grep -q "blocked_forbidden_operation:github_write_requested" "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/gate.json"
  grep -q "blocked_forbidden_operation:merge_requested" "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/gate.json"
  grep -q "blocked_forbidden_operation:approval_requested" "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/gate.json"
  grep -q "blocked_forbidden_operation:default_branch_push_requested" "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/gate.json"
  grep -q "blocked_forbidden_operation:force_push_requested" "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/gate.json"
  grep -q "blocked_forbidden_operation:tag_push_requested" "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/gate.json"
}

@test "v56 audit proves no implementation or raw secret exposure" {
  run bash "$TEST_TMP/repo/scripts/autospec-autonomous-v56-audit.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  python3 - "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/audit.json" <<'PY'
import json, sys
a=json.load(open(sys.argv[1]))
assert a["phase"] == "v56"
assert a["mode"] == "planning_only"
for key in ["network_attempted","github_write_attempted","git_push_attempted","pr_update_attempted","issue_publishing_attempted","merge_attempted","approval_attempted","self_approval_attempted","default_branch_push_attempted","force_push_attempted","tag_push_attempted","raw_secret_values_exposed"]:
    assert a[key] is False, key
PY
}

@test "v56 status refuses ready if audit artifact is missing" {
  bash "$TEST_TMP/repo/scripts/autospec-autonomous-v56-contract.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  rm -f "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/audit.json" "$TEST_TMP/repo/.autospec/reports/autonomous-v56-audit.json"
  run bash "$TEST_TMP/repo/scripts/autospec-v56-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -ne 0 ]
  grep -q "missing_audit_artifact" "$TEST_TMP/repo/.autospec/autonomy/v56/$RUN_ID/v56-status.json"
}

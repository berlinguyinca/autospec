#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

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
  git -C "$TEST_TMP/repo" checkout -b autospec/v61-mainline-freeze-test >/dev/null 2>&1
  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  for version in $(seq 26 60); do
    python3 "$TEST_TMP/repo/scripts/autospec-baseline-v25.py" --repo-root "$TEST_TMP/repo" --command "v${version}-supervisor" --prepare-only >/dev/null
    bash "$TEST_TMP/repo/scripts/autospec-v${version}-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  done
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "v61 writes mainline acceptance ledger and truth audit without overclaims" {
  run bash "$TEST_TMP/repo/scripts/autospec-v61-mainline-acceptance.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  run bash "$TEST_TMP/repo/scripts/autospec-v61-capability-truth-audit.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]

  [ -f "$TEST_TMP/repo/.autospec/baselines/v60-mainline-acceptance.json" ]
  [ -f "$TEST_TMP/repo/.autospec/baselines/v60-mainline-acceptance.md" ]
  [ -f "$TEST_TMP/repo/.autospec/audits/v61-capability-truth-audit.json" ]
  [ -f "$TEST_TMP/repo/.autospec/audits/v61-capability-truth-audit.md" ]

  python3 - "$TEST_TMP/repo/.autospec/baselines/v60-mainline-acceptance.json" "$TEST_TMP/repo/.autospec/audits/v61-capability-truth-audit.json" <<'PY'
import json, sys
ledger=json.load(open(sys.argv[1]))
audit=json.load(open(sys.argv[2]))
assert ledger["status"] == "accepted"
assert ledger["v60_status"] == "ready"
assert ledger["v61_status"] == "ready"
assert ledger["remote_write_readiness_overclaimed"] is False
assert ledger["real_canary_execution_claimed"] is False
assert len(ledger["phase_statuses"]) == 35
assert audit["status"] == "pass"
assert audit["overclaiming_prevented"] is True
assert audit["remote_write_canary_executed"] is False
assert audit["merge_capability_executed"] is False
assert audit["auto_merge_capability_executed"] is False
assert audit["self_approval_capability_executed"] is False
classifications={item["phase"]: set(item["classifications"]) for item in audit["capabilities"]}
assert "implemented" in classifications["v60"]
for phase in ["v26","v27","v29","v30","v34","v38","v43","v47","v57"]:
    assert "readiness_only" in classifications[phase], phase
    assert "requires_human_approval" in classifications[phase], phase
assert "mock_only" in classifications["v28"]
assert "local_only" in classifications["v40"]
assert "dry_run_only" in classifications["v51"]
PY
}

@test "v61 writes operator command catalog and golden paths" {
  bash "$TEST_TMP/repo/scripts/autospec-v61-operator-command-catalog.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-v61-golden-path-build.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-v61-golden-path-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null

  [ -f "$TEST_TMP/repo/.autospec/operator-command-catalog.json" ]
  [ -f "$TEST_TMP/repo/docs/operators/AUTOSPEC_COMMAND_CATALOG.md" ]
  [ -f "$TEST_TMP/repo/docs/operators/GOLDEN_PATH_AUTOTRADE.md" ]
  [ -f "$TEST_TMP/repo/docs/operators/GOLDEN_PATH_GENERIC_REPO.md" ]

  grep -q "Safety Classification" "$TEST_TMP/repo/docs/operators/AUTOSPEC_COMMAND_CATALOG.md"
  grep -q "Human approval boundary" "$TEST_TMP/repo/docs/operators/GOLDEN_PATH_AUTOTRADE.md"
  grep -q "Dry-run default" "$TEST_TMP/repo/docs/operators/GOLDEN_PATH_GENERIC_REPO.md"
  python3 - "$TEST_TMP/repo/.autospec/operator-command-catalog.json" <<'PY'
import json, sys
catalog=json.load(open(sys.argv[1]))
assert catalog["status"] == "written"
assert catalog["default_mode"] == "dry_run"
assert catalog["hidden_github_writes"] is False
assert len(catalog["commands"]) >= 10
assert any(c["safety_classification"] == "human_approval_required" for c in catalog["commands"])
assert any(c["safety_classification"] == "dry_run_safe" for c in catalog["commands"])
PY
}

@test "v61 boundary audits block hidden GitHub writes and approval overclaims" {
  bash "$TEST_TMP/repo/scripts/autospec-v61-human-approval-boundary-audit.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-v61-remote-write-boundary-audit.sh" --repo-root "$TEST_TMP/repo" >/dev/null

  python3 - "$TEST_TMP/repo/.autospec/audits/v61-human-approval-boundary-audit.json" "$TEST_TMP/repo/.autospec/audits/v61-remote-write-boundary-audit.json" <<'PY'
import json, sys
human=json.load(open(sys.argv[1]))
remote=json.load(open(sys.argv[2]))
assert human["status"] == "pass"
assert human["approval_capsule_required_for_remote_writes"] is True
assert human["self_approval_allowed"] is False
assert human["auto_merge_allowed"] is False
assert human["unapproved_real_write_allowed"] is False
assert remote["status"] == "pass"
assert remote["hidden_github_writes"] is False
assert remote["real_git_push_executed"] is False
assert remote["draft_pr_create_executed"] is False
assert remote["issue_publish_executed"] is False
assert remote["default_branch_push_executed"] is False
assert remote["force_push_executed"] is False
assert remote["tag_push_executed"] is False
PY
}

@test "v61 release candidate packet postmerge validation and status are ready" {
  bash "$TEST_TMP/repo/scripts/autospec-v61-mainline-acceptance.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-v61-capability-truth-audit.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-v61-operator-command-catalog.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-v61-golden-path-build.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-v61-human-approval-boundary-audit.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-v61-remote-write-boundary-audit.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-v61-release-candidate-pack.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-v61-postmerge-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  run bash "$TEST_TMP/repo/scripts/autospec-v61-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"v61 status: ready"* ]]

  for file in rc-summary validation-checklist known-limitations boundary-summary; do
    [ -f "$TEST_TMP/repo/.autospec/releases/v60-mainline-rc/$file.json" ]
    [ -f "$TEST_TMP/repo/.autospec/releases/v60-mainline-rc/$file.md" ]
  done
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v61-status.json" "$TEST_TMP/repo/.autospec/reports/v61-postmerge-validation.json" <<'PY'
import json, sys
status=json.load(open(sys.argv[1]))
post=json.load(open(sys.argv[2]))
assert status["status"] == "ready"
assert status["v60_mainline_acceptance_ledger_written"] is True
assert status["capability_truth_audit_written"] is True
assert status["operator_command_catalog_written"] is True
assert status["golden_path_docs_written"] is True
assert status["release_candidate_packet_written"] is True
assert status["human_approval_boundaries_explicit"] is True
assert status["remote_write_readiness_not_overclaimed"] is True
for key in ["auto_merge_attempted","self_approval_attempted","default_branch_push_attempted","github_write_attempted","scheduler_started","daemon_started","background_runner_started","raw_secret_values_exposed"]:
    assert status[key] is False, key
assert post["status"] == "pass"
assert post["platform_gates_unblocked"] is True
PY
}

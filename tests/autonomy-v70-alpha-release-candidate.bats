#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs"
  for v in 61 62 63 64 65 66 67 68 69; do bash "$TEST_TMP/repo/scripts/autospec-v${v}-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null; done
}

teardown() { rm -rf "$TEST_TMP"; }

@test "v70 packages alpha release candidate with governance artifacts" {
  for script in alpha-scope-lock alpha-release-candidate-pack pilot-program-matrix operator-runbook-build risk-register evidence-index alpha-acceptance-gate exit-criteria final-handoff; do
    bash "$TEST_TMP/repo/scripts/autospec-v70-$script.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  done
  run bash "$TEST_TMP/repo/scripts/autospec-v70-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"alpha_ready_with_accepted_warnings"* ]]
  [ -f "$TEST_TMP/repo/.autospec/releases/v70-alpha-rc/release-summary.md" ]
  [ -f "$TEST_TMP/repo/.autospec/releases/v70-alpha-rc/pilot-program-matrix.md" ]
  [ -f "$TEST_TMP/repo/.autospec/releases/v70-alpha-rc/operator-runbook.md" ]
  [ -f "$TEST_TMP/repo/.autospec/releases/v70-alpha-rc/risk-register.md" ]
  [ -f "$TEST_TMP/repo/.autospec/releases/v70-alpha-rc/final-handoff.md" ]
}

@test "v70 preserves accepted warnings and no hidden automation" {
  bash "$TEST_TMP/repo/scripts/autospec-v70-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v70-status.json" <<'PY'
import json, sys
s=json.load(open(sys.argv[1]))
assert s["status"] == "alpha_ready_with_accepted_warnings"
assert s["accepted_warnings"]
assert s["alpha_release_enables_hidden_automation"] is False
assert s["auto_merge_attempted"] is False
assert s["self_approval_attempted"] is False
assert s["default_branch_push_attempted"] is False
PY
}

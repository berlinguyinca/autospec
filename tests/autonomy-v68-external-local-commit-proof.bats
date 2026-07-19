#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs"
  for v in 61 62 63 64 65 66 67; do bash "$TEST_TMP/repo/scripts/autospec-v${v}-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null; done
}

teardown() { rm -rf "$TEST_TMP"; }

@test "v68 writes local commit ledger verifier revert drill and handoff" {
  for script in external-l2-target-prepare external-branch-safety-preflight external-local-commit-preflight external-cycle-write-commit external-commit-ledger-status external-commit-verifier external-revert-drill original-target-unchanged local-commit-handoff; do
    bash "$TEST_TMP/repo/scripts/autospec-v68-$script.sh" --repo-root "$TEST_TMP/repo" --allow-local-commit >/dev/null
  done
  run bash "$TEST_TMP/repo/scripts/autospec-v68-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v68/local-commit-ledger.json" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v68/commit-verifier.md" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v68/revert-drill.md" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v68/handoff.md" ]
}

@test "v68 blocks default branch push and remote write overclaims" {
  bash "$TEST_TMP/repo/scripts/autospec-v68-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  python3 - "$TEST_TMP/repo/.autospec/external-pilots/v68/local-commit-ledger.json" "$TEST_TMP/repo/.autospec/reports/autonomy-v68-status.json" <<'PY'
import json, sys
ledger=json.load(open(sys.argv[1]))
s=json.load(open(sys.argv[2]))
assert ledger["local_commits_created"] == 1
assert ledger["default_branch"] is False
assert s["git_push_attempted"] is False
assert s["github_write_attempted"] is False
PY
}

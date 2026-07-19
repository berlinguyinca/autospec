#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs"
  for v in 61 62 63 64 65 66; do bash "$TEST_TMP/repo/scripts/autospec-v${v}-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null; done
}

teardown() { rm -rf "$TEST_TMP"; }

@test "v67 writes one disposable docs evidence patch proof" {
  for script in external-disposable-prepare external-write-candidate-select external-scope-preflight external-apply-patch external-patch-verifier external-rollback external-rollback-verifier original-target-unchanged write-proof-handoff; do
    bash "$TEST_TMP/repo/scripts/autospec-v67-$script.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  done
  run bash "$TEST_TMP/repo/scripts/autospec-v67-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v67/write-candidate.md" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v67/apply-result.md" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v67/rollback-verification.md" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v67/original-target-unchanged.md" ]
  grep -q "candidate_count: \`1\`" "$TEST_TMP/repo/.autospec/external-pilots/v67/write-candidate.md"
}

@test "v67 reports no commits pushes network PRs or issue publishes" {
  bash "$TEST_TMP/repo/scripts/autospec-v67-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v67-status.json" <<'PY'
import json, sys
s=json.load(open(sys.argv[1]))
assert s["status"] == "ready"
for key in ["git_push_attempted","github_write_attempted","network_attempted","draft_pr_create_attempted","issue_publishing_attempted"]:
    assert s[key] is False, key
PY
}

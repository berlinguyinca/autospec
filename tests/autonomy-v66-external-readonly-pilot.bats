#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs"
  for v in 61 62 63 64 65; do bash "$TEST_TMP/repo/scripts/autospec-v${v}-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null; done
}

teardown() { rm -rf "$TEST_TMP"; }

@test "v66 writes external read-only pilot artifacts" {
  for script in external-target-register external-readonly-intake external-digital-twin-refresh external-risk-profile external-backlog-recommendations external-issue-draft-pack external-pilot-closeout original-target-unchanged; do
    bash "$TEST_TMP/repo/scripts/autospec-v66-$script.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  done
  run bash "$TEST_TMP/repo/scripts/autospec-v66-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v66/target-intake.md" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v66/digital-twin-summary.md" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v66/backlog-recommendations.md" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v66/issue-draft-pack.md" ]
  [ -f "$TEST_TMP/repo/.autospec/external-pilots/v66/closeout.md" ]
}

@test "v66 keeps issue drafts unpublished and original target unchanged" {
  bash "$TEST_TMP/repo/scripts/autospec-v66-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  grep -q "unpublished" "$TEST_TMP/repo/.autospec/external-pilots/v66/issue-draft-pack.md"
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v66-status.json" <<'PY'
import json, sys
s=json.load(open(sys.argv[1]))
assert s["status"] == "ready"
assert s["original_target_unchanged"] is True
assert s["issue_publishing_attempted"] is False
PY
}

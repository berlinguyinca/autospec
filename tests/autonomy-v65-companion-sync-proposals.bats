#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs"
  for v in 61 62 63 64; do bash "$TEST_TMP/repo/scripts/autospec-v${v}-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null; done
}

teardown() { rm -rf "$TEST_TMP"; }

@test "v65 writes companion proposal-only bridge artifacts" {
  for script in companion-inventory constitution-drift-audit baseline-drift-audit sync-proposal-plan companion-patch-bundle companion-compatibility-check manual-pr-packet proposal-quorum; do
    bash "$TEST_TMP/repo/scripts/autospec-v65-$script.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  done
  run bash "$TEST_TMP/repo/scripts/autospec-v65-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMP/repo/.autospec/companions/v65/constitution-drift-audit.md" ]
  [ -f "$TEST_TMP/repo/.autospec/companions/v65/baseline-drift-audit.md" ]
  [ -f "$TEST_TMP/repo/.autospec/companions/v65/sync-proposal-plan.md" ]
  [ -f "$TEST_TMP/repo/.autospec/companions/v65/manual-pr-packet.md" ]
  grep -q "no automatic PR creation" "$TEST_TMP/repo/.autospec/companions/v65/manual-pr-packet.md"
}

@test "v65 performs no companion repo write" {
  bash "$TEST_TMP/repo/scripts/autospec-v65-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v65-status.json" <<'PY'
import json, sys
s=json.load(open(sys.argv[1]))
assert s["status"] == "ready"
assert s["github_write_attempted"] is False
assert s["git_push_attempted"] is False
PY
}

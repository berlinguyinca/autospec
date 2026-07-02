#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs"
  git -C "$TEST_TMP/repo" init -q
  git -C "$TEST_TMP/repo" config user.email test@example.com
  git -C "$TEST_TMP/repo" config user.name Test
  git -C "$TEST_TMP/repo" add scripts docs >/dev/null 2>&1
  git -C "$TEST_TMP/repo" commit -qm init >/dev/null 2>&1 || true
  bash "$TEST_TMP/repo/scripts/autospec-v61-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null
}

teardown() { rm -rf "$TEST_TMP"; }

@test "v62 writes target registry matrix isolation and aggregate artifacts" {
  for script in target-registry-init target-register target-suitability-audit target-isolation-audit multirepo-pilot-plan multirepo-readonly-run pilot-matrix pilot-evidence-aggregate multirepo-handoff; do
    bash "$TEST_TMP/repo/scripts/autospec-v62-$script.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  done
  run bash "$TEST_TMP/repo/scripts/autospec-v62-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"v62 status: ready"* ]]
  [ -f "$TEST_TMP/repo/.autospec/multirepo/v62/target-registry.json" ]
  [ -f "$TEST_TMP/repo/.autospec/multirepo/v62/pilot-matrix.md" ]
  [ -f "$TEST_TMP/repo/.autospec/multirepo/v62/isolation-audit.md" ]
  [ -f "$TEST_TMP/repo/.autospec/multirepo/v62/evidence-aggregate.md" ]
  python3 - "$TEST_TMP/repo/.autospec/multirepo/v62/target-registry.json" "$TEST_TMP/repo/.autospec/reports/autonomy-v62-status.json" <<'PY'
import json, sys
registry=json.load(open(sys.argv[1]))
status=json.load(open(sys.argv[2]))
assert any(t["name"] == "autotrade" for t in registry["targets"])
assert any(t["name"] == "external-placeholder" for t in registry["targets"])
assert status["status"] == "ready"
assert status["github_write_attempted"] is False
assert status["background_runner_started"] is False
PY
}

@test "v62 blocks real network or GitHub write flags" {
  run bash "$TEST_TMP/repo/scripts/autospec-v62-status.sh" --repo-root "$TEST_TMP/repo" --allow-network
  [ "$status" -ne 0 ]
  [[ "$output" == *"blocked"* ]]
}

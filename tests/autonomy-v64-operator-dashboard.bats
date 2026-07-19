#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs"
  for v in 61 62 63; do bash "$TEST_TMP/repo/scripts/autospec-v${v}-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null; done
}

teardown() { rm -rf "$TEST_TMP"; }

@test "v64 renders a static dashboard with safety matrix" {
  for script in dashboard-data-build dashboard-static-render run-ledger-index control-plane-summary safety-matrix-render operator-dashboard-open-plan dashboard-verify; do
    bash "$TEST_TMP/repo/scripts/autospec-v64-$script.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  done
  run bash "$TEST_TMP/repo/scripts/autospec-v64-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMP/repo/.autospec/dashboard/v64/dashboard-data.json" ]
  [ -f "$TEST_TMP/repo/.autospec/dashboard/v64/index.html" ]
  [ -f "$TEST_TMP/repo/.autospec/dashboard/v64/control-plane-summary.md" ]
  [ -f "$TEST_TMP/repo/.autospec/dashboard/v64/safety-matrix.md" ]
  grep -q "Static artifact only" "$TEST_TMP/repo/.autospec/dashboard/v64/index.html"
}

@test "v64 dashboard does not start services or expose raw secrets" {
  bash "$TEST_TMP/repo/scripts/autospec-v64-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v64-status.json" <<'PY'
import json, sys
s=json.load(open(sys.argv[1]))
assert s["status"] == "ready"
assert s["daemon_started"] is False
assert s["scheduler_started"] is False
assert s["raw_secret_values_exposed"] is False
PY
}

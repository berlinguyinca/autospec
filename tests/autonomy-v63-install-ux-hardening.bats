#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs"
  bash "$TEST_TMP/repo/scripts/autospec-v61-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  bash "$TEST_TMP/repo/scripts/autospec-v62-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null
}

teardown() { rm -rf "$TEST_TMP"; }

@test "v63 writes install doctor onboarding and reproducibility outputs" {
  for script in install-doctor command-smoke operator-onboarding-pack release-bundle-repro-check docs-link-audit command-help-audit local-smoke-suite distribution-handoff; do
    bash "$TEST_TMP/repo/scripts/autospec-v63-$script.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  done
  run bash "$TEST_TMP/repo/scripts/autospec-v63-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [ -f "$TEST_TMP/repo/docs/operators/INSTALL_AND_DOCTOR.md" ]
  [ -f "$TEST_TMP/repo/docs/operators/COMMAND_SMOKE_GUIDE.md" ]
  [ -f "$TEST_TMP/repo/.autospec/distribution/v63/reproducibility-report.md" ]
  [ -f "$TEST_TMP/repo/.autospec/distribution/v63/operator-onboarding-pack.md" ]
  grep -q "no package installs" "$TEST_TMP/repo/docs/operators/INSTALL_AND_DOCTOR.md"
}

@test "v63 reports no hidden network or package operations" {
  bash "$TEST_TMP/repo/scripts/autospec-v63-status.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v63-status.json" <<'PY'
import json, sys
s=json.load(open(sys.argv[1]))
assert s["status"] == "ready"
assert s["network_attempted"] is False
assert s["package_operations"] is False
PY
}

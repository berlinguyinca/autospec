#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs" 2>/dev/null || mkdir -p "$TEST_TMP/repo/docs"
  cp -R "$REPO_ROOT/tests" "$TEST_TMP/repo/tests"
  mkdir -p "$TEST_TMP/repo/examples" "$TEST_TMP/repo/.autospec/reports"
  printf '# Fixture AutoSpec\n\nA fixture README.\n' > "$TEST_TMP/repo/README.md"
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "v25 requested validation commands generate baseline artifacts" {
  run bash "$TEST_TMP/repo/scripts/autospec-spec-coverage.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  run bash "$TEST_TMP/repo/scripts/autospec-release-validation.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  run bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"V25_BASELINE_READY=true"* ]]

  [ -f "$TEST_TMP/repo/.autospec/reports/repository-audit.md" ]
  [ -f "$TEST_TMP/repo/.autospec/spec-index.json" ]
  [ -f "$TEST_TMP/repo/.autospec/spec-index.md" ]
  [ -f "$TEST_TMP/repo/.autospec/reports/dependency-validation.md" ]
  [ -f "$TEST_TMP/repo/.autospec/reports/documentation-coverage.md" ]
  [ -f "$TEST_TMP/repo/.autospec/reports/cli-audit.md" ]
  [ -f "$TEST_TMP/repo/.autospec/reports/test-matrix.md" ]
  [ -f "$TEST_TMP/repo/.autospec/baselines/performance.json" ]
  [ -f "$TEST_TMP/repo/.autospec/baselines/quality.json" ]
  [ -f "$TEST_TMP/repo/.autospec/baselines/v25-baseline.json" ]
  [ -f "$TEST_TMP/repo/.autospec/releases/v25.md" ]
  [ -f "$TEST_TMP/repo/.autospec/reports/autonomy-v25-status.json" ]
}

@test "v25 status is ready and reports clean safety proof" {
  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  run bash "$TEST_TMP/repo/scripts/autospec-v25-status.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"V25_BASELINE_READY=true"* ]]

  python3 - "$TEST_TMP/repo/.autospec/reports/autonomy-v25-status.json" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
assert data["status"] == "ready"
assert data["V25_BASELINE_READY"] is True
for key in [
    "network_attempted",
    "github_write_attempted",
    "git_push_attempted",
    "draft_pr_create_attempted",
    "issue_publishing_attempted",
    "merge_attempted",
    "approval_attempted",
    "self_approval_attempted",
    "default_branch_push_attempted",
    "raw_secret_values_exposed",
]:
    assert data[key] is False, key
assert data["scheduler"] == "absent"
assert data["daemon"] == "absent"
assert data["background_runner"] == "absent"
assert data["external_ai"] == "disabled_by_default"
PY
}

@test "v25 spec inventory assigns each spec to exactly one state" {
  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  python3 - "$TEST_TMP/repo/.autospec/spec-index.json" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
states = {"implemented", "scaffolded", "validated", "deferred", "experimental", "superseded"}
assert data["summary"]["duplicate_assignments"] == 0
for item in data["specs"]:
    assert item["state"] in states
    assert isinstance(item["path"], str) and item["path"]
PY
}

@test "v25 dependency graph is acyclic and release validation has no blockers" {
  bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo" >/dev/null
  python3 - "$TEST_TMP/repo/.autospec/reports/dependency-validation.json" "$TEST_TMP/repo/.autospec/reports/release-validation.json" <<'PY'
import json, sys
dep = json.load(open(sys.argv[1]))
rel = json.load(open(sys.argv[2]))
assert dep["acyclic"] is True
assert dep["blockers"] == []
assert rel["status"] == "pass"
assert rel["blockers"] == []
PY
}

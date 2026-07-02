#!/usr/bin/env bats

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/repo"
  cp -R "$REPO_ROOT/scripts" "$TEST_TMP/repo/scripts"
  cp -R "$REPO_ROOT/docs" "$TEST_TMP/repo/docs" 2>/dev/null || mkdir -p "$TEST_TMP/repo/docs"
  printf '# Fixture
' > "$TEST_TMP/repo/README.md"
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "v15 release freeze compatibility baseline" {
  run bash "$TEST_TMP/repo/scripts/autospec-baseline-validation.sh" --repo-root "$TEST_TMP/repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"V25_BASELINE_READY=true"* ]]
}

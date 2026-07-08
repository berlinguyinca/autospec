#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi


REPO_ROOT="${BATS_TEST_DIRNAME}/.."
SCRIPT="$REPO_ROOT/scripts/autospec-control-plane.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  GH_LOG="$TEST_TMP/gh.log"
  mkdir -p "$TEST_TMP/bin"
  cat > "$TEST_TMP/bin/gh" <<'GH'
#!/usr/bin/env bash
printf 'gh %s\n' "$*" >> "$GH_LOG"
exit 42
GH
  chmod +x "$TEST_TMP/bin/gh"
  export GH_LOG
  export PATH="$TEST_TMP/bin:$PATH"
}

teardown() {
  rm -rf "$TEST_TMP"
}

@test "help documents bootstrap dry-run" {
  run bash "$SCRIPT" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"bootstrap --dry-run"* ]]
  [[ "$output" == *"autospec-governance"* ]]
}

@test "governance bootstrap dry-run renders scaffold and does not run gh" {
  run bash "$SCRIPT" bootstrap --dry-run --owner berlinguyinca --governance-repo autospec-governance
  [ "$status" -eq 0 ]
  [[ "$output" == *"autospec-governance/"* ]]
  [[ "$output" == *"policies/open-source-maintainer-default.yml"* ]]
  [[ "$output" == *"rules/security.yml"* ]]
  [[ "$output" == *"schemas/policy.schema.json"* ]]
  [[ "$output" == *"fixtures/projects/open-source-cli.yml"* ]]
  [[ "$output" == *"tests/policy-schema.bats"* ]]
  [[ "$output" == *"docs/policy-authoring.md"* ]]
  [[ ! -s "$GH_LOG" ]]
}

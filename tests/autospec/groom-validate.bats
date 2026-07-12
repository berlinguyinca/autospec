#!/usr/bin/env bats
# tests/autospec/groom-validate.bats — template-groom validator tests.

setup() {
  TMP="$(mktemp -d)"; SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/groom-validate.sh"
  export AUTOSPEC_LINT_ISSUE_BIN="$TMP/lint.sh"
}
teardown() { rm -rf "$TMP"; }

@test "passes when linter is clean" {
  printf '#!/usr/bin/env bash\nexit 0\n' > "$TMP/lint.sh"; chmod +x "$TMP/lint.sh"
  printf 'well-formed body' > "$TMP/body"
  run bash "$SCRIPT" "$TMP/body"; [ "$status" -eq 0 ]; echo "$output" | jq -e '.ok == true'
}
@test "fails and surfaces findings when linter rejects" {
  printf '#!/usr/bin/env bash\necho "MISSING: ### Primary smoke test" >&2\nexit 1\n' > "$TMP/lint.sh"; chmod +x "$TMP/lint.sh"
  printf 'bad body' > "$TMP/body"
  run bash "$SCRIPT" "$TMP/body"; [ "$status" -eq 1 ]
  echo "$output" | jq -e '.ok == false and (.findings|length > 0)'
}

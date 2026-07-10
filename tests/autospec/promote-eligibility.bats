setup() { TMP="$(mktemp -d)"; SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/promote-eligibility.sh"; }
teardown() { rm -rf "$TMP"; }
mkbody() { printf '%s' "$1" > "$TMP/b"; }

@test "clear single-file bug fix is eligible" {
  mkbody "fix: guard set -eu abort in loop.sh line 1250; repro: run conductor with empty backlog, observe crash. Expected: no crash."
  run bash "$SCRIPT" "$TMP/b" --labels "bug"
  echo "$output" | jq -e '.decision == "eligible"'
}
@test "epic label routes to epic" {
  mkbody "big umbrella of work across many subsystems"
  run bash "$SCRIPT" "$TMP/b" --labels "epic,enhancement"
  echo "$output" | jq -e '.decision == "epic"'
}
@test "thin/ambiguous body holds (fail-closed)" {
  mkbody "make it better"
  run bash "$SCRIPT" "$TMP/b" --labels ""
  echo "$output" | jq -e '.decision == "hold"'
}
@test "unresolvable dependency holds" {
  mkbody "fix: something concrete and actionable here with detail. Depends on #999999"
  GH_NONEXISTENT=1 run bash "$SCRIPT" "$TMP/b" --labels "bug"
  echo "$output" | jq -e '.decision == "hold" and (.reason|test("depend";"i"))'
}

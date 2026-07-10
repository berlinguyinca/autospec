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
@test "body-text epic marker is not eligible" {
  mkbody "fix: this is an epic effort spanning the whole system; see loop.sh for the crash. Expected: no crash."
  run bash "$SCRIPT" "$TMP/b" --labels "bug"
  echo "$output" | jq -e '.decision != "eligible"'
}
@test "epicenter substring does not false-trigger epic marker" {
  mkbody "fix: recenter the epicenter marker in map.sh; repro: open map, observe offset. Expected: centered."
  run bash "$SCRIPT" "$TMP/b" --labels "bug"
  echo "$output" | jq -e '.decision == "eligible"'
}
@test "threads --repo into the Depends-on existence check (cross-repo grooming)" {
  mkdir -p "$TMP/bin"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
# Dependency exists ONLY when queried against the target repo o/r.
case "$*" in
  *"issue view"*"--repo o/r"*) exit 0 ;;
  *) exit 1 ;;
esac
SH
  chmod +x "$TMP/bin/gh"
  export PATH="$TMP/bin:$PATH"
  mkbody "fix: concrete actionable change in map.sh; repro: open map. Expected: fixed. Depends on #5"
  run bash "$SCRIPT" "$TMP/b" --labels "bug" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.decision == "eligible"'
}
@test "without --repo the cross-repo dependency cannot be confirmed → hold (fail-closed)" {
  mkdir -p "$TMP/bin"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
case "$*" in
  *"issue view"*"--repo o/r"*) exit 0 ;;
  *) exit 1 ;;
esac
SH
  chmod +x "$TMP/bin/gh"
  export PATH="$TMP/bin:$PATH"
  mkbody "fix: concrete actionable change in map.sh; repro: open map. Expected: fixed. Depends on #5"
  run bash "$SCRIPT" "$TMP/b" --labels "bug"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.decision == "hold"'
}

#!/usr/bin/env bats
# Control mirroring is ADDITIVE ONLY: a locally-paused repo must never be
# un-paused by the board.

setup() {
  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-control-mirror.sh"
  export GH_CALLS="$TMP/gh.log"; : > "$GH_CALLS"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_CALLS"
case "$*" in
  *"issue view"*"--json labels"*)
    printf '%s' "${GH_CONTROL_LABELS:-[]}"
    ;;
  *"issue list"*"--label autospec:project-board-marker"*)
    if [ "${GH_MARKER_EXIT:-0}" -ne 0 ]; then
      exit "${GH_MARKER_EXIT}"
    fi
    printf '%s' "${GH_MARKER_JSON:-[]}"
    ;;
  *"issue list"*)
    printf '%s' "${GH_REPO_LABELS:-[]}"
    ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
}
teardown() { rm -rf "$TMP"; }

@test "a project-level pause is mirrored into every fleet repo" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a,o/b --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored | length == 2'
  grep -q 'o/a' "$GH_CALLS"
  grep -q 'o/b' "$GH_CALLS"
}

@test "mirroring never removes a label a repo set for itself" {
  export GH_CONTROL_LABELS='[]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  ! grep -q -- '--remove-label' "$GH_CALLS"
}

@test "a control issue outside the allowlist disables mirroring" {
  export GH_CONTROL_LABELS='[{"name":"autospec:stop"}]'
  run bash "$SCRIPT" --control-issue evil/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'code_health:project_board_repo_out_of_scope'
  ! grep -q -- '--add-label' "$GH_CALLS"
}

@test "an unset control issue is a no-op, not an error" {
  run bash "$SCRIPT" --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored == []'
}

@test "only the four reserved labels are mirrored" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"},{"name":"bug"},{"name":"autospec:steer"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  echo "$output" | jq -e '[.mirrored[].label] | sort == ["autospec:pause","autospec:steer"]'
  ! grep -q -- '--add-label bug' "$GH_CALLS"
}

@test "a target repo outside the allowlist is skipped" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a,evil/b --allowlist 'o/*'
  echo "$output" | jq -e '.skipped | length == 1'
  ! grep -q 'evil/b' "$GH_CALLS"
}

@test "literal allowlist matching: regex-looking patterns do not act as regex" {
  export GH_CONTROL_LABELS='[{"name":"autospec:stop"}]'
  # Control issue is kept in-scope via an exact literal entry so each case
  # below isolates literal-vs-regex behavior for the TARGET repo pattern.

  run bash "$SCRIPT" --control-issue o/ctl#1 --repos oa/r --allowlist 'o/ctl,o.*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.skipped | length == 1'

  run bash "$SCRIPT" --control-issue o/ctl#1 --repos "oa/r" --allowlist 'o/ctl,o(a|b)/r'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.skipped | length == 1'

  run bash "$SCRIPT" --control-issue o/ctl#1 --repos "any/thing" --allowlist 'o/ctl,.*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.skipped | length == 1'
}

@test "literal matching handles a repo name containing regex metacharacters" {
  export GH_CONTROL_LABELS='[{"name":"autospec:stop"}]'
  run bash "$SCRIPT" --control-issue 'o/ctl#1' --repos 'o/re.po(1)' --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored | length == 1'
}

@test "no gh call ever names a repo outside the allowlist, for the control issue either" {
  export GH_CONTROL_LABELS='[{"name":"autospec:stop"}]'
  run bash "$SCRIPT" --control-issue evil/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  ! grep -q 'evil' "$GH_CALLS"
}

@test "degenerate input: missing --repos does not crash" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored == [] and .skipped == []'
}

@test "degenerate input: malformed gh output does not crash" {
  export GH_CONTROL_LABELS='not-json'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored == [] and .skipped == []'
}

# --- marker find-failure handling (Finding 1) ---------------------------
# A transient/errored marker lookup must be treated as "could not
# determine", never as "found nothing" — the latter would create a
# duplicate marker issue on every flaky cycle.

@test "a gh non-zero exit during marker lookup skips the repo and creates nothing" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_EXIT=1
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored == [] and (.skipped | length == 1)'
  ! grep -q -- 'issue create' "$GH_CALLS"
}

@test "malformed JSON during marker lookup skips the repo and creates nothing" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='not-json'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored == [] and (.skipped | length == 1)'
  ! grep -q -- 'issue create' "$GH_CALLS"
}

@test "a valid-but-non-array marker lookup (e.g. an error object) skips and creates nothing" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='{"message":"API rate limit exceeded"}'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored == [] and (.skipped | length == 1)'
  ! grep -q -- 'issue create' "$GH_CALLS"
}

@test "a valid empty marker lookup creates the marker issue" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='[]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored | length == 1'
  grep -q -- 'issue create' "$GH_CALLS"
}

@test "multiple marker matches: picks the lowest issue number and never creates" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='[{"number":42},{"number":7},{"number":99}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored | length == 1'
  grep -q -- 'issue edit 7 --repo o/a --add-label autospec:pause' "$GH_CALLS"
  ! grep -q -- 'issue create' "$GH_CALLS"
}

# --- marker exists-branch coverage (Finding 2) ---------------------------

@test "an existing marker issue is edited in place, never recreated" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='[{"number":55}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored | length == 1'
  grep -q -- 'issue edit 55 --repo o/a --add-label autospec:pause' "$GH_CALLS"
  ! grep -q -- 'issue create' "$GH_CALLS"
}

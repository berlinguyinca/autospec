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
  *"issue edit"*"--add-label"*)
    if [ "${GH_ADD_LABEL_EXIT:-0}" -ne 0 ]; then
      exit "${GH_ADD_LABEL_EXIT}"
    fi
    printf '' ;;
  *"issue create"*)
    if [ "${GH_ISSUE_CREATE_EXIT:-0}" -ne 0 ]; then
      exit "${GH_ISSUE_CREATE_EXIT}"
    fi
    printf '' ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
}
teardown() { rm -rf "$TMP"; }

MARKER_TITLE='[autospec] project-board control relay (do not edit manually)'

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

@test "a valid empty marker lookup creates the marker issue, labeled so it is findable next cycle" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='[]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored | length == 1'
  grep -q -- 'issue create' "$GH_CALLS"
  # This is the property the whole marker design depends on: if the
  # create call ever stops attaching MARKER_LABEL, the marker becomes
  # unfindable on the next cycle and the script would create a fresh
  # duplicate every single time. Pin it directly.
  grep -q -- 'issue create.*--label autospec:project-board-marker' "$GH_CALLS"
}

@test "multiple marker matches: picks the lowest issue number and never creates" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='[{"number":42,"title":"'"$MARKER_TITLE"'"},{"number":7,"title":"'"$MARKER_TITLE"'"},{"number":99,"title":"'"$MARKER_TITLE"'"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored | length == 1'
  grep -q -- 'issue edit 7 --repo o/a --add-label autospec:pause' "$GH_CALLS"
  ! grep -q -- 'issue create' "$GH_CALLS"
}

# --- marker exists-branch coverage (Finding 2) ---------------------------

@test "an existing marker issue is edited in place, never recreated" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='[{"number":55,"title":"'"$MARKER_TITLE"'"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored | length == 1'
  grep -q -- 'issue edit 55 --repo o/a --add-label autospec:pause' "$GH_CALLS"
  ! grep -q -- 'issue create' "$GH_CALLS"
}

# --- adoption gate: label alone is not proof of ownership (Finding 2) ---
# A candidate found by --label is only adopted if its title is an exact
# match for MARKER_TITLE too. A foreign issue that merely carries the
# marker label (mislabeled by a human, copied while triaging, ...) must
# never be edited, and must never trigger creating a second marker either
# — it is skipped with a distinct reason.

@test "a label-match with the correct title is adopted" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='[{"number":55,"title":"'"$MARKER_TITLE"'"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored | length == 1'
  grep -q -- 'issue edit 55 --repo o/a --add-label autospec:pause' "$GH_CALLS"
  ! grep -q -- 'issue create' "$GH_CALLS"
}

@test "a label-match with a foreign title is not adopted, edited, or duplicated" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='[{"number":12,"title":"Fix flaky test"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored == [] and (.skipped | length == 1)'
  # $output-dependent check above must come before the `run` below, since
  # `run` overwrites $output.
  run grep -q -- 'issue edit 12' "$GH_CALLS"
  [ "$status" -ne 0 ]
  ! grep -q -- 'issue create' "$GH_CALLS"
}

@test "a foreign-title skip surfaces a distinct reason in the skipped array" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='[{"number":12,"title":"Fix flaky test"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.skipped[0].reason == "marker_label_title_mismatch"'
}

# --- I4: a failing gh call must never be reported as mirrored (Plan B) ---
# The `--add-label` call used to end in `|| true`, with the label appended
# to `mirrored` regardless of whether the API call actually succeeded — the
# worst possible lie for a stop signal an operator believes propagated.

@test "a failing gh add-label on an existing marker is NOT reported as mirrored" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='[{"number":55,"title":"'"$MARKER_TITLE"'"}]'
  export GH_ADD_LABEL_EXIT=1
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored == []'
  echo "$output" | jq -e '.failed | length == 1'
  echo "$output" | jq -e '.failed[0].repo == "o/a" and .failed[0].label == "autospec:pause"'
  grep -q -- 'issue edit 55 --repo o/a --add-label autospec:pause' "$GH_CALLS"
}

@test "a failing gh issue create is NOT reported as mirrored" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='[]'
  export GH_ISSUE_CREATE_EXIT=1
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored == []'
  echo "$output" | jq -e '.failed | length == 1'
  echo "$output" | jq -e '.failed[0].reason == "gh_issue_create_failed"'
}

@test "a successful gh call is still reported as mirrored (control: failed stays empty)" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  export GH_MARKER_JSON='[{"number":55,"title":"'"$MARKER_TITLE"'"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored | length == 1'
  echo "$output" | jq -e '.failed == []'
}

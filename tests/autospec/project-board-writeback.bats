#!/usr/bin/env bats
# Coverage for scripts/project-board-writeback.sh — fail-open board field mutation.

setup() {
  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-writeback.sh"
  export GH_CALLS="$TMP/gh-calls.log"; : > "$GH_CALLS"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_CALLS"
case "$*" in
  *"auth status"*) [ "${GH_SCOPE_OK:-1}" = "1" ] && printf "Token scopes: 'project', 'repo'\n" || printf "Token scopes: 'repo'\n" ;;
  *"item-edit"*)   exit "${GH_EDIT_RC:-0}" ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  cat > "$TMP/plan.json" <<'JSON'
{"project":{"id":"PVT_1"},
 "fields":{"autospec_state":{"id":"PVTSSF_1","options":{"Ready":"opt_ready","Done":"opt_done"}}},
 "items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"autospec_state":"Blocked"}]}
JSON
}
teardown() { rm -rf "$TMP"; }

# ── Brief's six tests ────────────────────────────────────────────────────────

@test "writes the mapped single-select option id" {
  run bash "$SCRIPT" --plan "$TMP/plan.json" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  grep -q 'item-edit' "$GH_CALLS"
  grep -q 'opt_ready' "$GH_CALLS"
  grep -q 'PVTSSF_1' "$GH_CALLS"
}

@test "skips a no-op write when the item already holds that state" {
  # The shared plan.json fixture never lists "Blocked" as a board option, so
  # asserting idempotence against it would pass via the "no matching option"
  # skip branch rather than the real already-in-state branch. Give this test
  # its own fixture where Blocked IS a valid option and the item already
  # holds it, so it genuinely exercises the idempotence check.
  jq '.fields.autospec_state.options = {"Ready":"opt_ready","Blocked":"opt_blocked"}' \
     "$TMP/plan.json" > "$TMP/blocked.json"
  run bash "$SCRIPT" --plan "$TMP/blocked.json" --item PVTI_a --state Blocked
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'already in state Blocked'
  ! grep -q 'item-edit' "$GH_CALLS"
}

@test "a gh failure is fail-open and emits the code_health marker" {
  GH_EDIT_RC=1 run bash "$SCRIPT" --plan "$TMP/plan.json" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'code_health:project_board_writeback_failed'
}

@test "a token without the project scope disables write-back with one warning" {
  GH_SCOPE_OK=0 run bash "$SCRIPT" --plan "$TMP/plan.json" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  ! grep -q 'item-edit' "$GH_CALLS"
  echo "$output" | grep -q 'project scope'
}

@test "a board without an AutoSpec state field is skipped, never created" {
  jq 'del(.fields.autospec_state)' "$TMP/plan.json" > "$TMP/nofield.json"
  run bash "$SCRIPT" --plan "$TMP/nofield.json" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  ! grep -q 'item-edit' "$GH_CALLS"
  ! grep -q 'field-create' "$GH_CALLS"
}

@test "an unknown state name is skipped, never invented as an option" {
  run bash "$SCRIPT" --plan "$TMP/plan.json" --item PVTI_a --state Nonsense
  [ "$status" -eq 0 ]
  ! grep -q 'item-edit' "$GH_CALLS"
}

# ── Controller amendment: candidate resolution ──────────────────────────────

@test "a canonical state resolves to the p1-style option name" {
  jq '.fields.autospec_state.options = {"Ready":"o_r","In progress":"o_ip","Done":"o_d"}' \
     "$TMP/plan.json" > "$TMP/p1style.json"
  run bash "$SCRIPT" --plan "$TMP/p1style.json" --item PVTI_a --state Implementation
  [ "$status" -eq 0 ]
  grep -q 'o_ip' "$GH_CALLS"
}

@test "the p2-style option still wins when both candidates exist" {
  jq '.fields.autospec_state.options = {"Implementation":"o_impl","In progress":"o_ip"}' \
     "$TMP/plan.json" > "$TMP/both.json"
  run bash "$SCRIPT" --plan "$TMP/both.json" --item PVTI_a --state Implementation
  grep -q 'o_impl' "$GH_CALLS"
  ! grep -q 'o_ip' "$GH_CALLS"
}

@test "a canonical state with no matching candidate skips without creating an option" {
  jq '.fields.autospec_state.options = {"Ready":"o_r"}' "$TMP/plan.json" > "$TMP/thin.json"
  run bash "$SCRIPT" --plan "$TMP/thin.json" --item PVTI_a --state Testing
  [ "$status" -eq 0 ]
  ! grep -q 'item-edit' "$GH_CALLS"
  ! grep -q 'field-create' "$GH_CALLS"
}

# ── Degenerate-input guards ──────────────────────────────────────────────────

@test "missing --plan entirely exits 0 with a reason" {
  run bash "$SCRIPT" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'missing --plan'
  ! grep -q 'item-edit' "$GH_CALLS"
}

@test "a --plan file that is not JSON exits 0 with a reason" {
  printf 'not json at all {' > "$TMP/bad.json"
  run bash "$SCRIPT" --plan "$TMP/bad.json" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'not valid JSON'
  ! grep -q 'item-edit' "$GH_CALLS"
}

@test "a plan with no .fields is skipped, never crashes" {
  echo '{"project":{"id":"PVT_1"},"items":[{"item_id":"PVTI_a","autospec_state":"Blocked"}]}' > "$TMP/nofields.json"
  run bash "$SCRIPT" --plan "$TMP/nofields.json" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  ! grep -q 'item-edit' "$GH_CALLS"
}

@test "a plan with no .items is skipped, never crashes" {
  echo '{"project":{"id":"PVT_1"},"fields":{"autospec_state":{"id":"F1","options":{"Ready":"o_r"}}}}' > "$TMP/noitems.json"
  run bash "$SCRIPT" --plan "$TMP/noitems.json" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'not found in plan'
  ! grep -q 'item-edit' "$GH_CALLS"
}

@test "an --item id not present in the plan is skipped" {
  run bash "$SCRIPT" --plan "$TMP/plan.json" --item PVTI_ZZZ --state Ready
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'not found in plan'
  ! grep -q 'item-edit' "$GH_CALLS"
}

@test "an empty --state is skipped" {
  run bash "$SCRIPT" --plan "$TMP/plan.json" --item PVTI_a --state ""
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'missing --state'
  ! grep -q 'item-edit' "$GH_CALLS"
}

@test "a null .fields.autospec_state.options is skipped, never crashes" {
  jq '.fields.autospec_state.options = null' "$TMP/plan.json" > "$TMP/nulloptions.json"
  run bash "$SCRIPT" --plan "$TMP/nulloptions.json" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  ! grep -q 'item-edit' "$GH_CALLS"
}

@test "a wrong-type .fields.autospec_state.options is skipped, never crashes" {
  jq '.fields.autospec_state.options = ["Ready","Done"]' "$TMP/plan.json" > "$TMP/arrayoptions.json"
  run bash "$SCRIPT" --plan "$TMP/arrayoptions.json" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  ! grep -q 'item-edit' "$GH_CALLS"
}

# ── Finding I3: the token-scope probe is cached per run, not repeated per item ─

@test "gh auth status is probed at most once across a multi-item run against the same plan" {
  # The caller (autonomous-promote-open-issues.sh) invokes this script once
  # per item, always with the SAME --plan file for the whole cycle (measured:
  # 80 `gh auth status` calls in one p2 cycle before this fix). Simulate a
  # multi-item run by invoking this script three times against the same
  # shared plan.json, and assert the underlying gh binary was asked for
  # 'auth status' at most once across all three.
  # The shared plan.json fixture's item already holds "Blocked" — pick three
  # target states that all genuinely differ from it, so every call is a real
  # edit rather than tripping the (separately tested) idempotence skip.
  jq '.fields.autospec_state.options = {"Ready":"opt_ready","Done":"opt_done","Testing":"opt_testing"}' \
     "$TMP/plan.json" > "$TMP/multi.json"

  run bash "$SCRIPT" --plan "$TMP/multi.json" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  run bash "$SCRIPT" --plan "$TMP/multi.json" --item PVTI_a --state Done
  [ "$status" -eq 0 ]
  run bash "$SCRIPT" --plan "$TMP/multi.json" --item PVTI_a --state Testing
  [ "$status" -eq 0 ]

  auth_calls="$(grep -c 'auth status' "$GH_CALLS")"
  [ "$auth_calls" -eq 1 ]
  # The cache must not have suppressed real work: all three edits still fired.
  edit_calls="$(grep -c 'item-edit' "$GH_CALLS")"
  [ "$edit_calls" -eq 3 ]
}

@test "a fresh plan file (new run) re-probes gh auth status" {
  run bash "$SCRIPT" --plan "$TMP/plan.json" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  jq '.' "$TMP/plan.json" > "$TMP/plan2.json"
  run bash "$SCRIPT" --plan "$TMP/plan2.json" --item PVTI_a --state Ready
  [ "$status" -eq 0 ]
  auth_calls="$(grep -c 'auth status' "$GH_CALLS")"
  [ "$auth_calls" -eq 2 ]
}

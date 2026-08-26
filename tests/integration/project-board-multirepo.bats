#!/usr/bin/env bats
# The load-bearing claim this whole plan exists to prove: a cross-repo
# `Blocked by: o/up#N` dependency holds repo B's promotion until repo A's
# blocking issue closes. Everything else in the project-board feature
# (resolver, normalize, deps, promoter, fleet) exists so THIS is true.
#
# Uses the REAL project-board-normalize.sh and project-board-deps.sh — only
# the resolver (AUTOSPEC_BOARD_RESOLVE_SCRIPT) is stubbed, so the board
# content is under test control but the dependency-resolution logic that
# decides readiness is the genuine, unmodified pipeline.
#
# SAFETY: `gh` and AUTOSPEC_GROOM_SAFETY_BIN are both stubbed to local
# scripts under a per-test mktemp -d; the real GitHub API and the real
# `autospec` binary (present on PATH) are never invoked. No --apply run in
# this file uses anything but the stubs.

bats_require_minimum_version 1.5.0

setup() {
  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/autonomous-promote-open-issues.sh"
  FIX="${BATS_TEST_DIRNAME}/../fixtures/project-board"

  export AUTOSPEC_BOARD_NORMALIZE_SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-normalize.sh"
  export AUTOSPEC_BOARD_DEPS_SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-deps.sh"
  export AUTOSPEC_BOARD_RESOLVE_SCRIPT="$TMP/resolve.sh"
  export AUTOSPEC_PROJECT_BOARD_URL="https://github.com/orgs/o/projects/1"
  export AUTOSPEC_PROJECT_BOARD_ALLOWLIST="o/*"
  export AUTOSPEC_PROJECT_BOARD_TTL=0
  export AUTOSPEC_STATE_DIR="$TMP/state"

  # Never let the GROOM_LIST (issue-triage) path contribute candidates —
  # every test in this file is exercising the BOARD source exclusively.
  export AUTOSPEC_GROOM_LIST_SCRIPT="$TMP/list.sh"
  cat > "$TMP/list.sh" <<'SH'
#!/usr/bin/env bash
printf '%s' '{"candidates":[],"skipped":[]}'
SH
  chmod +x "$TMP/list.sh"

  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "gh $*" >> "$GH_CALLS"
case "$*" in *"issue list"*) printf '[]' ;; *) printf '' ;; esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  export GH_CALLS="$TMP/gh-calls.log"; : > "$GH_CALLS"

  # Rust safety-authority stub (real `autospec` IS on PATH — this seam keeps
  # every --apply test in this file from ever shelling out to it).
  cat > "$TMP/bin/safety.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "autospec $*" >> "$GH_CALLS"
if [ "${1:-}" != "issue" ] || [ "${2:-}" != "promote" ]; then
  exit 41
fi
printf '%s\n' '{"safety":{"decision":"pass"},"auto-implement":true,"eligible":true}'
SH
  chmod +x "$TMP/bin/safety.sh"
  export AUTOSPEC_GROOM_SAFETY_BIN="$TMP/bin/safety.sh"
}
teardown() { rm -rf "$TMP"; }

board() {
  cat > "$TMP/resolve.sh" <<SH
#!/usr/bin/env bash
cat <<'JSON'
$1
JSON
SH
  chmod +x "$TMP/resolve.sh"
}

UPSTREAM_OPEN='{"project":{},"fields":{},"repos":["o/up","o/down"],"items":[
 {"item_id":"PVTI_up","repo":"o/up","number":1,"state":"open","labels":[],"body":"Blocked by: none."},
 {"item_id":"PVTI_dn","repo":"o/down","number":5,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: o/up#1.\n"}]}'

UPSTREAM_CLOSED='{"project":{},"fields":{},"repos":["o/up","o/down"],"items":[
 {"item_id":"PVTI_up","repo":"o/up","number":1,"state":"closed","labels":[],"body":"Blocked by: none."},
 {"item_id":"PVTI_dn","repo":"o/down","number":5,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: o/up#1.\n"}]}'

# ── Core scenario: repo B is held by repo A's open blocker, then released ──

@test "the upstream repo is ready first" {
  board "$UPSTREAM_OPEN"
  run bash "$SCRIPT" --repo o/up
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.ready == 1'
  echo "$output" | jq -e '.board.promotable == [1]'
}

@test "the downstream repo is held while the upstream issue is open" {
  board "$UPSTREAM_OPEN"
  run bash "$SCRIPT" --repo o/down
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.ready == 0'
  echo "$output" | jq -e '.board.promotable == []'
}

@test "the downstream repo becomes ready once the upstream issue closes" {
  board "$UPSTREAM_CLOSED"
  run bash "$SCRIPT" --repo o/down
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.ready == 1'
  echo "$output" | jq -e '.board.promotable == [5]'
}

@test "without --apply, neither the held nor the released run mutates anything (proven from the gh log, not the return value)" {
  board "$UPSTREAM_OPEN"
  run bash "$SCRIPT" --repo o/down
  [ "$status" -eq 0 ]
  board "$UPSTREAM_CLOSED"
  run bash "$SCRIPT" --repo o/down
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.dry == true'
  ! grep -qE 'issue edit|issue comment|label create' "$GH_CALLS"
}

# ── Allowlist: an out-of-scope repo is never promotable, and gh never hears
#    its name (checked against the stub's own argument log, not a return
#    value or a mocked assertion) ──────────────────────────────────────────

@test "an item in a repo outside the allowlist is never promotable and no gh call ever names that repo" {
  board '{"project":{},"fields":{},"repos":["evil/x"],"items":[
   {"item_id":"PVTI_e","repo":"evil/x","number":1,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  run bash "$SCRIPT" --repo evil/x
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.ready == 0'
  echo "$output" | jq -e '.board.out_of_scope | length == 1'
  ! grep -q 'evil/x' "$GH_CALLS"
}

# ── Regression guard: deps_unresolvable must never be promotable, even when
#    it is the ONLY ready-looking sibling. Modeled on the real p2 board's
#    Phase 5.5 final-audit item (#80): it declares its dependency in prose
#    ("Blocked by the implementation and acceptance portfolio IW-WB-001
#    through IW-WB-078") with no parseable `#N`, so project-board-deps.sh
#    marks it deps_unresolvable rather than silently treating it as
#    unblocked. This is the ordering guarantee that stops autospec
#    implementing the final audit before the 78 issues it audits. Uses the
#    REAL pinned p2 fixture body verbatim — not a paraphrase — so this
#    cannot drift from the live board. ──────────────────────────────────────

@test "deps_unresolvable item #80 from the real p2 fixture is never promotable while #1 is" {
  item1="$(jq -c '.items[] | select(.content.number == 1) |
    {item_id: .id, repo: .content.repository, number: .content.number,
     state: "open", labels: (.labels // []), body: .content.body}' "$FIX/p2-items.json")"
  item80="$(jq -c '.items[] | select(.content.number == 80) |
    {item_id: .id, repo: .content.repository, number: .content.number,
     state: "open", labels: (.labels // []), body: .content.body}' "$FIX/p2-items.json")"
  [ -n "$item1" ]
  [ -n "$item80" ]
  repo="$(printf '%s' "$item1" | jq -r '.repo')"
  plan="$(jq -n --argjson a "$item1" --argjson b "$item80" --arg repo "$repo" \
    '{project:{},fields:{},repos:[$repo],items:[$a,$b]}')"
  export AUTOSPEC_PROJECT_BOARD_ALLOWLIST="${repo%%/*}/*"
  board "$plan"
  run bash "$SCRIPT" --repo "$repo"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.ready == 1'
  echo "$output" | jq -e '[.board.promotable[]?] | index(1) != null'
  echo "$output" | jq -e '[.board.promotable[]?] | index(80) == null'
}

# ── Shared per-cycle budget caps board promotions and REPORTS the
#    truncation rather than silently dropping the rest. ────────────────────

@test "the shared per-cycle budget caps board promotions in one repo and reports truncation" {
  export AUTOSPEC_GROOMING_POLICY=auto
  export AUTOSPEC_GROOMING_MAX_ISSUES=1
  board '{"project":{},"fields":{},"repos":["o/up","o/down"],"items":[
   {"item_id":"PVTI_1","repo":"o/up","number":1,"state":"open","labels":[],"body":"Blocked by: none."},
   {"item_id":"PVTI_2","repo":"o/up","number":2,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  run bash "$SCRIPT" --repo o/up --apply
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.promotable == [1,2]'
  echo "$output" | jq -e '(.promoted | length) == 1'
  echo "$output" | jq -e '.promoted[0] == 1'
  echo "$output" | jq -e '.board.truncated == 1'
  ! grep -q 'o/down' "$GH_CALLS"
}

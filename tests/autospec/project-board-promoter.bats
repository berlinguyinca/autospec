#!/usr/bin/env bats
# Coverage for the Tier 1.5 board source in scripts/autonomous-promote-open-issues.sh.

bats_require_minimum_version 1.5.0

setup() {
  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/autonomous-promote-open-issues.sh"
  export AUTOSPEC_BOARD_RESOLVE_SCRIPT="$TMP/resolve.sh"
  export AUTOSPEC_BOARD_NORMALIZE_SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-normalize.sh"
  export AUTOSPEC_BOARD_DEPS_SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-deps.sh"
  export AUTOSPEC_PROJECT_BOARD_URL="https://github.com/orgs/InferWeave/projects/2"
  export AUTOSPEC_PROJECT_BOARD_ALLOWLIST="o/*"
  # Isolate the on-disk board cache per test — the cache path is keyed by URL
  # only, and every test in this file reuses the same URL.
  export AUTOSPEC_STATE_DIR="$TMP/state"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "gh $*" >> "$GH_CALLS"
case "$*" in *"issue list"*) printf '[]' ;; *) printf '' ;; esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  export GH_CALLS="$TMP/gh-calls.log"; : > "$GH_CALLS"

  # ── Rust safety-authority stub (Task 8/10 apply-path tests) ─────────────────
  # `autospec` is a real binary on this machine's PATH; without a stub the
  # apply-path board loop would shell out to it (and it, or a live gh call,
  # could reach the real GitHub API). Default: always passes admission.
  cat > "$TMP/bin/safety.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "autospec $*" >> "$GH_CALLS"
if [ "${1:-}" != "issue" ] || [ "${2:-}" != "promote" ]; then
  exit 41
fi
json="$SAFETY_JSON"
[ -n "$json" ] || json='{"safety":{"decision":"pass"},"auto-implement":true,"eligible":true}'
printf '%s\n' "$json"
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

@test "without --apply the board source mutates nothing" {
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.dry == true'
  ! grep -q 'issue edit' "$GH_CALLS"
}

@test "a ready board item is reported promotable" {
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  run bash "$SCRIPT" --repo o/r
  echo "$output" | jq -e '.board.ready == 1'
}

@test "a blocked board item is not promotable" {
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":1,"state":"open","labels":[],"body":"Blocked by: none."},{"item_id":"PVTI_b","repo":"o/r","number":5,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: #1.\n"}]}'
  run bash "$SCRIPT" --repo o/r
  echo "$output" | jq -e '[.board.promotable[]?] | index(5) == null'
}

@test "an item outside the allowlist is skipped as out_of_scope" {
  export AUTOSPEC_PROJECT_BOARD_ALLOWLIST="safe/*"
  board '{"project":{},"fields":{},"repos":["evil/repo"],"items":[{"item_id":"PVTI_x","repo":"evil/repo","number":1,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  run bash "$SCRIPT" --repo safe/r
  echo "$output" | jq -e '.board.out_of_scope | length == 1'
  ! grep -q 'evil/repo' "$GH_CALLS"
}

@test "only items in the conductor's own repo are considered" {
  board '{"project":{},"fields":{},"repos":["o/r","o/other"],"items":[{"item_id":"PVTI_a","repo":"o/other","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  run bash "$SCRIPT" --repo o/r
  echo "$output" | jq -e '.board.ready == 0'
}

@test "an unset board url leaves the envelope untouched" {
  unset AUTOSPEC_PROJECT_BOARD_URL
  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e 'has("filed") and has("promoted")'
}

@test "a resolver failure yields a dry board signal, never a crash" {
  cat > "$TMP/resolve.sh" <<'SH'
#!/usr/bin/env bash
exit 4
SH
  chmod +x "$TMP/resolve.sh"
  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.ready == 0'
}

# ── Controller ruling #3: null priority must never sort/promote as highest ──

@test "an item with no priority label ranks behind prioritized items, never first" {
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[
    {"item_id":"PVTI_nopri","repo":"o/r","number":1,"state":"open","labels":[],"body":"Blocked by: none."},
    {"item_id":"PVTI_hi","repo":"o/r","number":2,"state":"open","labels":["priority:high"],"body":"Blocked by: none."},
    {"item_id":"PVTI_crit","repo":"o/r","number":3,"state":"open","labels":["priority:critical"],"body":"Blocked by: none."}
  ]}'
  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.promotable == [3,2,1]'
}

# ── Controller ruling #4: deps_unresolvable is never promotable, even with an
#    empty blocked_by. Modeled on the real p2-board Phase 5.5 final-audit item
#    (#80), whose body declares a dependency marker over unparseable prose
#    instead of `#N` references. ─────────────────────────────────────────────

@test "an item with an unparseable declared dependency is never promotable" {
  # Verbatim body (not a paraphrase) so this cannot drift from the real
  # board: the whole point is that item #80's REAL prose keeps the final
  # audit from promoting ahead of the 78 issues it audits.
  audit_body="$(jq -r '.items[] | select(.content.number == 80) | .content.body' "${BATS_TEST_DIRNAME}/../fixtures/project-board/p2-items.json")"
  [ -n "$audit_body" ]
  board_fixture="$(jq -n --arg body "$audit_body" '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_audit","repo":"o/r","number":80,"state":"open","labels":[],"body":$body}]}')"
  board "$board_fixture"
  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.ready == 0'
  echo "$output" | jq -e '[.board.promotable[]?] | index(80) == null'
}


# ── Task 10: write-back into the promoter's lifecycle points ────────────────

@test "promotion writes Ready back to the board" {
  export AUTOSPEC_BOARD_WRITEBACK_SCRIPT="$TMP/wb.sh"
  printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$*" >> "$TMP/wb.log"\n' > "$TMP/wb.sh"
  chmod +x "$TMP/wb.sh"; export TMP
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --repo o/r --apply
  grep -q -- '--state Ready' "$TMP/wb.log"
}

@test "a blocked item writes Blocked back to the board" {
  export AUTOSPEC_BOARD_WRITEBACK_SCRIPT="$TMP/wb.sh"
  printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$*" >> "$TMP/wb.log"\n' > "$TMP/wb.sh"
  chmod +x "$TMP/wb.sh"; export TMP
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":1,"state":"open","labels":[],"body":"Blocked by: none."},{"item_id":"PVTI_b","repo":"o/r","number":5,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: #1.\n"}]}'
  AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --repo o/r --apply
  grep -q -- '--item PVTI_b --state Blocked' "$TMP/wb.log"
}

@test "write-back never runs without --apply" {
  export AUTOSPEC_BOARD_WRITEBACK_SCRIPT="$TMP/wb.sh"
  printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$*" >> "$TMP/wb.log"\n' > "$TMP/wb.sh"
  chmod +x "$TMP/wb.sh"; export TMP; : > "$TMP/wb.log"
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  run bash "$SCRIPT" --repo o/r
  [ ! -s "$TMP/wb.log" ]
}

@test "a write-back failure does not fail the promotion" {
  export AUTOSPEC_BOARD_WRITEBACK_SCRIPT="$TMP/wb.sh"
  printf '#!/usr/bin/env bash\nexit 1\n' > "$TMP/wb.sh"; chmod +x "$TMP/wb.sh"
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
}

@test "write-back never fires when grooming policy is off" {
  export AUTOSPEC_BOARD_WRITEBACK_SCRIPT="$TMP/wb.sh"
  printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$*" >> "$TMP/wb.log"\n' > "$TMP/wb.sh"
  chmod +x "$TMP/wb.sh"; export TMP; : > "$TMP/wb.log"
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  AUTOSPEC_GROOMING_POLICY=off run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  [ ! -s "$TMP/wb.log" ]
}

# ── NEW-1: the auth-scope probe cache must not outlive its own run ─────────
# The earlier fix cached project-board-writeback.sh's `gh auth status` probe
# on disk beside --plan (BOARD_CACHE), which is the PERSISTENT, URL-keyed
# board cache — reused across separate promoter invocations, not a fresh
# per-run temp file. That permanently froze a stale scope answer on the host.
# The fix moves the cache into the promoter's own in-process shell variable
# (AUTOSPEC_PROJECT_BOARD_AUTH_OK, exported to each child), so it never
# touches disk and cannot survive past this process. These two tests invoke
# the REAL project-board-writeback.sh (not the wb.sh stub used elsewhere in
# this file) so the probe/cache path actually executes end to end.

real_writeback_gh_stub() {
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "gh $*" >> "$GH_CALLS"
case "$*" in
  *"issue list"*)   printf '[]' ;;
  *"auth status"*)  printf "Token scopes: 'project', 'repo'\n" ;;
  *"item-edit"*)    exit 0 ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"
}

@test "NEW-1: gh auth status is probed at most once per run across multiple board items" {
  export AUTOSPEC_BOARD_WRITEBACK_SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-writeback.sh"
  real_writeback_gh_stub
  # Two items in one cycle: PVTI_a promotes to Ready, PVTI_b (blocked on #1)
  # writes Blocked — two separate board_writeback calls, same run.
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":1,"state":"open","labels":[],"body":"Blocked by: none.","autospec_state":"Blocked"},{"item_id":"PVTI_b","repo":"o/r","number":5,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: #1.\n","autospec_state":"Blocked"}]}'
  AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  auth_calls="$(grep -c 'auth status' "$GH_CALLS")"
  [ "$auth_calls" -eq 1 ]
}

@test "NEW-1: a fresh promoter run re-probes gh auth status rather than reusing a previous run's cached answer" {
  export AUTOSPEC_BOARD_WRITEBACK_SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-writeback.sh"
  real_writeback_gh_stub
  # Same URL/AUTOSPEC_STATE_DIR across both invocations (unchanged from
  # setup()), so --plan resolves to the SAME persistent, URL-keyed cache file
  # both times — exactly the production scenario the bug depended on.
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none.","autospec_state":"Blocked"}]}'

  AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]

  auth_calls="$(grep -c 'auth status' "$GH_CALLS")"
  [ "$auth_calls" -eq 2 ]
}

# ── Admission control: board promotions share ONE per-cycle budget with the
#    GROOM_LIST path (budget.max_issues_per_cycle), never a second one ─────

@test "a board with more ready items than the budget promotes exactly budget, in ranked order, and reports the truncation" {
  export AUTOSPEC_GROOMING_MAX_ISSUES=2
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[
    {"item_id":"PVTI_1","repo":"o/r","number":1,"state":"open","labels":[],"body":"Blocked by: none."},
    {"item_id":"PVTI_2","repo":"o/r","number":2,"state":"open","labels":["priority:critical"],"body":"Blocked by: none."},
    {"item_id":"PVTI_3","repo":"o/r","number":3,"state":"open","labels":["priority:high"],"body":"Blocked by: none."},
    {"item_id":"PVTI_4","repo":"o/r","number":4,"state":"open","labels":[],"body":"Blocked by: none."}
  ]}'
  AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  # ranked order is [2,3,1,4] (critical, high, then null-priority by number);
  # a budget of 2 promotes exactly the top two of that ranking.
  echo "$output" | jq -e '.board.promotable == [2,3,1,4]'
  echo "$output" | jq -e '(.promoted | sort) == [2,3]'
  echo "$output" | jq -e '.board.truncated == 2'
}

@test "a shared budget already partly consumed by the grooming path leaves only the remainder for the board" {
  export AUTOSPEC_GROOMING_MAX_ISSUES=3
  export AUTOSPEC_GROOM_LIST_SCRIPT="$TMP/list.sh"
  cat > "$TMP/list.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' '{"candidates":[{"number":101,"title":"t","class":"unlabeled"},{"number":102,"title":"t","class":"unlabeled"}],"skipped":[]}'
SH
  chmod +x "$TMP/list.sh"

  export AUTOSPEC_GROOM_ELIGIBILITY_SCRIPT="$TMP/elig.sh"
  cat > "$TMP/elig.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' '{"decision":"eligible","reason":"stub"}'
SH
  chmod +x "$TMP/elig.sh"

  # gh: `issue view` returns a pre-classified fixture (finalize_ready then
  # skips classification); everything else still logs to GH_CALLS.
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "gh $*" >> "$GH_CALLS"
case "$*" in
  *"issue view"*) printf '{"number":%s,"title":"t","body":"b","labels":[{"name":"ctx:64k"},{"name":"reasoning:medium"}]}' "$3" ;;
  *"issue list"*) printf '[]' ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"

  board '{"project":{},"fields":{},"repos":["o/r"],"items":[
    {"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."},
    {"item_id":"PVTI_b","repo":"o/r","number":6,"state":"open","labels":[],"body":"Blocked by: none."}
  ]}'
  AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  # grooming already promoted 2 of the shared budget of 3, leaving exactly 1
  # for the board (item 5, the lower ranked number) — item 6 is truncated.
  echo "$output" | jq -e '(.promoted | sort) == [5,101,102]'
  echo "$output" | jq -e '.board.truncated == 1'
}

@test "a zero board budget promotes nothing and reports every ready item as truncated" {
  export AUTOSPEC_GROOMING_MAX_ISSUES=0
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.promotable == [5]'
  echo "$output" | jq -e '.board.truncated == 1'
  echo "$output" | jq -e '(.promoted | length) == 0'
}

# ── C1: concurrent workers must never crash the whole Tier 1.5 pass ─────────
# board_plan() used to write every worker's resolved plan to the SAME fixed
# "$_cache.tmp" path before mv-ing it into place. The winner's mv succeeded;
# every other worker's mv then failed (source already renamed away) and
# aborted the whole script under `set -eu`, before the grooming loop even
# ran — a fleet-wide silent Tier 1.5 stall. A small sleep in the resolver
# stub widens the race window so N workers reliably overlap at the mv step
# regardless of process-spawn jitter, without weakening the assertion (a
# false pass would still need real overlap; this only removes flakiness).

@test "C1: N concurrent workers racing the same board cache all exit 0 with a valid envelope" {
  cat > "$TMP/resolve.sh" <<'SH'
#!/usr/bin/env bash
sleep 0.2
cat <<'JSON'
{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}
JSON
SH
  chmod +x "$TMP/resolve.sh"

  n=6
  for i in $(seq 1 "$n"); do
    # Under bats' `set -e`, "cmd; echo $?" never reaches the echo when cmd
    # fails — the subshell aborts right after the failing command. Capture
    # the status through `||` (a checked context) instead.
    ( rc=0
      bash "$SCRIPT" --repo o/r >"$TMP/out.$i" 2>"$TMP/err.$i" || rc="$?"
      printf '%s' "$rc" >"$TMP/rc.$i"
    ) &
  done
  wait

  fail_count=0
  for i in $(seq 1 "$n"); do
    rc="$(cat "$TMP/rc.$i")"
    if [ "$rc" -ne 0 ]; then
      fail_count=$((fail_count + 1))
      echo "worker $i rc=$rc stderr=$(cat "$TMP/err.$i")"
    fi
    [ "$rc" -eq 0 ]
    jq -e 'type == "object"' "$TMP/out.$i" >/dev/null
  done
  [ "$fail_count" -eq 0 ]
}

@test "a malformed budget falls back to the existing default, not zero admission" {
  cat > "$TMP/config.sh" <<'SH'
#!/usr/bin/env bash
case "$*" in
  *"policy"*) printf 'auto\n' ;;
  *"budget.max_issues_per_cycle"*) printf 'not-a-number\n' ;;
  *) printf '\n' ;;
esac
SH
  chmod +x "$TMP/config.sh"
  export AUTOSPEC_GROOM_CONFIG_SCRIPT="$TMP/config.sh"
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  run bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.truncated == 0'
  echo "$output" | jq -e '(.promoted | index(5)) != null'
}

# ── I6: cycle handling must not be a silent dead end ────────────────────────

@test "I6: a dependency cycle surfaces the code_health marker on stderr" {
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[
    {"item_id":"PVTI_1","repo":"o/r","number":1,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: #2.\n"},
    {"item_id":"PVTI_2","repo":"o/r","number":2,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: #1.\n"}
  ]}'
  run --separate-stderr bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.ready == 0'
  [[ "$stderr" == *"code_health:project_board_dependency_cycle"* ]]
}

@test "I6: cycle participants are reported in the envelope and never promotable" {
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[
    {"item_id":"PVTI_1","repo":"o/r","number":1,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: #2.\n"},
    {"item_id":"PVTI_2","repo":"o/r","number":2,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: #1.\n"}
  ]}'
  run --separate-stderr bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '(.board.cycle_participants | sort) == [1,2]'
  echo "$output" | jq -e '[.board.promotable[]?] | length == 0'
}

@test "I6: --apply labels cycle participants autospec:needs-human and audits, without promoting them" {
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[
    {"item_id":"PVTI_1","repo":"o/r","number":1,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: #2.\n"},
    {"item_id":"PVTI_2","repo":"o/r","number":2,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: #1.\n"}
  ]}'
  AUTOSPEC_GROOMING_POLICY=auto run --separate-stderr bash "$SCRIPT" --repo o/r --apply
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '(.promoted | length) == 0'
  grep -q -- 'issue edit 1 --repo o/r --add-label autospec:needs-human' "$GH_CALLS"
  grep -q -- 'issue edit 2 --repo o/r --add-label autospec:needs-human' "$GH_CALLS"
}

@test "I6: without --apply, cycle participants are never labeled" {
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[
    {"item_id":"PVTI_1","repo":"o/r","number":1,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: #2.\n"},
    {"item_id":"PVTI_2","repo":"o/r","number":2,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: #1.\n"}
  ]}'
  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  ! grep -q 'add-label autospec:needs-human' "$GH_CALLS"
}

@test "I6: an acyclic board reports zero cycle participants" {
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.cycle_participants == []'
}

# ── M4: `.board.promotable` must mean what it says ──────────────────────────

@test "M4: a ready item already labeled autospec:needs-human is excluded from promotable" {
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":["autospec:needs-human"],"body":"Blocked by: none."}]}'
  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.ready == 1'
  echo "$output" | jq -e '[.board.promotable[]?] | index(5) == null'
}

@test "M4: a ready item already labeled in-progress-by-bot is excluded from promotable" {
  board '{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":["in-progress-by-bot"],"body":"Blocked by: none."}]}'
  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.board.promotable[]?] | index(5) == null'
}

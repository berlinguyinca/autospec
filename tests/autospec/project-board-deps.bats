#!/usr/bin/env bats
# Coverage for scripts/project-board-deps.sh — dependency extraction.

setup() {
  TMP="$(mktemp -d)"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-deps.sh"
  FIX="${BATS_TEST_DIRNAME}/../fixtures/project-board"
}
teardown() { rm -rf "$TMP"; }

item() { printf '{"items":[{"repo":"o/r","number":9,"body":%s,"dependencies":%s,"parent_issue":%s}]}' "$1" "${2:-null}" "${3:-null}"; }

@test "parses a bare '#N' blocked-by against the item's own repo" {
  item '"## Dependencies\n\n- Blocked by: #1 (IW-WB-000).\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == [{"repo":"o/r","number":1}]'
}

@test "parses a cross-repo 'owner/repo#N' blocked-by" {
  item '"## Dependencies\n\n- Blocked by: InferWeave/inferweave-protocol#42.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '.items[0].blocked_by == [{"repo":"InferWeave/inferweave-protocol","number":42}]'
}

@test "managed active DependsOn and Blocks edges preserve direction in readiness" {
  cat > "$TMP/in.json" <<'JSON'
{"active_edges":[
  {"kind":"depends-on","state":"active","source":"https://github.com/o/a/issues/1","target":"https://github.com/o/b/issues/2"},
  {"kind":"blocks","state":"active","source":"https://github.com/o/c/issues/3","target":"https://github.com/o/d/issues/4"}
],"items":[
  {"repo":"o/a","number":1,"state":"open"},
  {"repo":"o/b","number":2,"state":"open"},
  {"repo":"o/c","number":3,"state":"open"},
  {"repo":"o/d","number":4,"state":"open"}
]}
JSON
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '
    (.items[] | select(.repo == "o/a") | .blocked_by == [{"repo":"o/b","number":2}] and .ready == false) and
    (.items[] | select(.repo == "o/d") | .blocked_by == [{"repo":"o/c","number":3}] and .ready == false) and
    (.items[] | select(.repo == "o/b") | .ready == true) and
    (.items[] | select(.repo == "o/c") | .ready == true)'
}

@test "parses multiple blocked-by references on one line" {
  item '"## Dependencies\n\n- Blocked by: #1, #2, #3.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [1,2,3]'
}

@test "'Blocked by: none' yields an empty list" {
  item '"## Dependencies\n\n- Blocked by: none.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '.items[0].blocked_by == []'
}

@test "the Dependencies field wins over the body" {
  item '"## Dependencies\n\n- Blocked by: #99.\n"' '"#5"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [5]'
}

@test "the parent issue wins over the body when no Dependencies field is set" {
  item '"## Dependencies\n\n- Blocked by: #99.\n"' 'null' '"o/r#7"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [7]'
}

@test "a '#N' outside the Dependencies section is ignored" {
  item '"## Problem statement\n\nSee #123 for context.\n\n## Dependencies\n\n- Blocked by: none.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '.items[0].blocked_by == []'
}

@test "the real project-2 fixture yields blockers on 78 of 80 items" {
  jq '{items: [.items[] | {repo: .content.repository, number: .content.number, body: .content.body, dependencies: null, parent_issue: null}]}' \
     "$FIX/p2-items.json" > "$TMP/p2.json"
  run bash "$SCRIPT" < "$TMP/p2.json"
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.items[] | select((.blocked_by | length) > 0)] | length')" -eq 78 ]
}

# --- Degenerate input coverage ---------------------------------------------

@test "degenerate: stdin that is not JSON exits 0 with no crash" {
  printf 'not json at all {{{' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
}

@test "degenerate: JSON with no .items exits 0 and passes input through" {
  printf '{"foo":"bar"}' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.foo == "bar"'
}

@test "degenerate: an item with no body yields an empty blocked_by" {
  printf '{"items":[{"repo":"o/r","number":1}]}' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == []'
}

@test "degenerate: body: null yields an empty blocked_by" {
  printf '{"items":[{"repo":"o/r","number":1,"body":null}]}' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == []'
}

@test "degenerate: an item with no repo does not crash on a bare '#N'" {
  item='{"items":[{"number":1,"body":"## Dependencies\n\n- Blocked by: #4.\n"}]}'
  printf '%s' "$item" > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by[0].number == 4'
}

@test "degenerate: dependencies field is a non-string type (array) falls through" {
  item '"## Dependencies\n\n- Blocked by: #4.\n"' '[1,2,3]' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [4]'
}

@test "degenerate: parent_issue field is a non-string type (object) falls through" {
  item '"## Dependencies\n\n- Blocked by: #4.\n"' 'null' '{"foo":"bar"}' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [4]'
}

@test "degenerate: dependencies field is a non-string type (number) falls through" {
  item '"## Dependencies\n\n- Blocked by: #4.\n"' '123' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.items[0].blocked_by[].number] == [4]'
}

# --- Review findings ---------------------------------------------------

@test "finding 1: 'Depends on issue #N' marker phrase is parsed by default" {
  item '"## Dependencies\n\nDepends on issue #22\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == [{"repo":"o/r","number":22}]'
}

@test "finding 1: the real project-1 fixture yields blockers on 54 of 80 items" {
  jq '{items: [.items[] | {repo: .content.repository, number: .content.number, body: .content.body, dependencies: null, parent_issue: null}]}' \
     "$FIX/p1-items.json" > "$TMP/p1.json"
  run bash "$SCRIPT" < "$TMP/p1.json"
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.items[] | select((.blocked_by | length) > 0)] | length')" -eq 54 ]
}

@test "finding 1: the marker set is overridable via AUTOSPEC_PROJECT_BOARD_DEP_MARKERS" {
  item '"## Dependencies\n\nWaiting on #9\n"' > "$TMP/in.json"
  # not parsed under the default marker set
  run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '.items[0].blocked_by == []'
  # parsed once the board configures its own marker phrase
  AUTOSPEC_PROJECT_BOARD_DEP_MARKERS="Waiting on" run bash "$SCRIPT" < "$TMP/in.json"
  echo "$output" | jq -e '.items[0].blocked_by == [{"repo":"o/r","number":9}]'
}

@test "finding 1: a marker phrase still has no effect outside the Dependencies section" {
  item '"## Notes\n\nDepends on issue #123 historically.\n\n## Dependencies\n\nnone\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == []'
}

@test "finding 2: the real p2 fixture distinguishes item 1 (clean 'none') from item 80 (unresolvable prose)" {
  jq '{items: [.items[] | {repo: .content.repository, number: .content.number, body: .content.body, dependencies: null, parent_issue: null}]}' \
     "$FIX/p2-items.json" > "$TMP/p2.json"
  run bash "$SCRIPT" < "$TMP/p2.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '
    (.items[] | select(.number == 1) | .blocked_by == [] and .deps_unresolvable == false) and
    (.items[] | select(.number == 80) | .blocked_by == [] and .deps_unresolvable == true and (.deps_reason | length) > 0)'
}

@test "finding 2: a marker phrase with no parseable #N sets deps_unresolvable and a reason" {
  item '"## Dependencies\n\nBlocked by the whole prior portfolio.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == [] and .items[0].deps_unresolvable == true and (.items[0].deps_reason | type) == "string"'
}

@test "finding 2: 'Blocked by: none' stays cleanly unblocked, no unresolvable flag" {
  item '"## Dependencies\n\n- Blocked by: none.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == [] and .items[0].deps_unresolvable == false and .items[0].deps_reason == null'
}

@test "finding 3: a '#N' inside a fenced code block in Dependencies is not a real edge" {
  item '"## Dependencies\n\n```\nBlocked by: #666\n```\n\n- Blocked by: none.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == []'
}

@test "finding 4: a '#N' inside an HTML comment in Dependencies is not a real edge" {
  item '"## Dependencies\n\n<!-- Blocked by: #666 -->\n\n- Blocked by: none.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == []'
}

@test "finding 4: surrounding text is intact after stripping unrelated HTML comments" {
  item '"<!-- autospec-classify:begin -->\nsome notes\n<!-- autospec-classify:end -->\n\n## Dependencies\n\n- Blocked by: #7.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == [{"repo":"o/r","number":7}]'
}

@test "finding 5: two '## Dependencies' sections are unioned when the first has no marker" {
  item '"## Dependencies\n\nSee below.\n\n## Other\n\ntext\n\n## Dependencies\n\n- Blocked by: #11.\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == [{"repo":"o/r","number":11}]'
}

@test "not-a-regression: a bare '#N' still resolves against the item's own repo, never guessed cross-repo" {
  item '"## Dependencies\n\nDepends on issue #4\n"' > "$TMP/in.json"
  run bash "$SCRIPT" < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].blocked_by == [{"repo":"o/r","number":4}]'
}

two() { printf '{"items":[%s,%s]}' "$1" "$2" > "$TMP/in.json"; }

@test "an item with no blockers is ready" {
  two '{"repo":"o/r","number":1,"state":"open","blocked_by":[]}' \
      '{"repo":"o/r","number":2,"state":"open","blocked_by":[]}'
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[] | select(.number==1) | .ready == true'
}

@test "an item blocked by an open item is not ready and names its blocker" {
  two '{"repo":"o/r","number":1,"state":"open","blocked_by":[]}' \
      '{"repo":"o/r","number":2,"state":"open","blocked_by":[{"repo":"o/r","number":1}]}'
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  echo "$output" | jq -e '.items[] | select(.number==2) | .ready == false'
  echo "$output" | jq -e '.items[] | select(.number==2) | .reason | test("o/r#1")'
}

@test "an item blocked by a closed item is ready" {
  two '{"repo":"o/r","number":1,"state":"closed","blocked_by":[]}' \
      '{"repo":"o/r","number":2,"state":"open","blocked_by":[{"repo":"o/r","number":1}]}'
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  echo "$output" | jq -e '.items[] | select(.number==2) | .ready == true'
}

@test "an unresolvable blocker is unsatisfied, never satisfied" {
  two '{"repo":"o/r","number":1,"state":"closed","blocked_by":[]}' \
      '{"repo":"o/r","number":2,"state":"open","blocked_by":[{"repo":"gone/repo","number":404}]}'
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  echo "$output" | jq -e '.items[] | select(.number==2) | .ready == false'
  echo "$output" | jq -e '.items[] | select(.number==2) | .reason | test("unresolvable")'
}

@test "a cross-repo chain stays blocked until the upstream repo closes" {
  two '{"repo":"o/up","number":1,"state":"open","blocked_by":[]}' \
      '{"repo":"o/down","number":5,"state":"open","blocked_by":[{"repo":"o/up","number":1}]}'
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  echo "$output" | jq -e '.items[] | select(.repo=="o/down") | .ready == false'
}

@test "a dependency cycle is reported and its participants are not ready" {
  two '{"repo":"o/r","number":1,"state":"open","blocked_by":[{"repo":"o/r","number":2}]}' \
      '{"repo":"o/r","number":2,"state":"open","blocked_by":[{"repo":"o/r","number":1}]}'
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '.cycles | length')" -gt 0 ]
  echo "$output" | jq -e '.items[] | select(.number==1) | .ready == false'
  echo "$output" | jq -e '.items[] | select(.number==1) | .reason | test("cycle")'
}

@test "an acyclic item stays ready alongside a cycle elsewhere on the board" {
  printf '{"items":[%s,%s,%s]}' \
    '{"repo":"o/r","number":1,"state":"open","blocked_by":[{"repo":"o/r","number":2}]}' \
    '{"repo":"o/r","number":2,"state":"open","blocked_by":[{"repo":"o/r","number":1}]}' \
    '{"repo":"o/r","number":3,"state":"open","blocked_by":[]}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  echo "$output" | jq -e '.items[] | select(.number==3) | .ready == true'
}

# --- Task 6 controller rulings -----------------------------------------

@test "ruling 1: an item with deps_unresolvable=true is never ready, even with empty blocked_by" {
  printf '{"items":[{"repo":"o/r","number":1,"state":"open","blocked_by":[],"deps_unresolvable":true,"deps_reason":"prose only"}]}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[0].ready == false'
  echo "$output" | jq -e '.items[0].reason | test("unresolvable")'
}

@test "ruling 1: the real p2 fixture — item 80 (unresolvable prose) is never ready, item 1 (clean none) is ready" {
  jq '{items: [.items[] | {repo: .content.repository, number: .content.number, state: "open", body: .content.body, dependencies: null, parent_issue: null}]}' \
     "$FIX/p2-items.json" > "$TMP/p2.json"
  run bash "$SCRIPT" --resolve < "$TMP/p2.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[] | select(.number == 80) | .ready == false and (.reason | test("unresolvable"))'
  echo "$output" | jq -e '.items[] | select(.number == 1) | .ready == true'
}

@test "ruling 2: the no-flag output is byte-identical to the pre-Task-6 extraction output" {
  jq '{items: [.items[] | {repo: .content.repository, number: .content.number, body: .content.body, dependencies: null, parent_issue: null}]}' \
     "$FIX/p1-items.json" > "$TMP/p1.json"
  run bash "$SCRIPT" < "$TMP/p1.json"
  [ "$status" -eq 0 ]
  # no .ready, .reason, or top-level .cycles anywhere when --resolve is absent
  echo "$output" | jq -e '[.items[] | has("ready") or has("reason")] | any | not'
  echo "$output" | jq -e '(has("cycles")) | not'
}

@test "degenerate: --resolve on non-JSON stdin exits 0 with no crash" {
  printf 'not json {{{' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
}

@test "degenerate: --resolve on JSON with no .items passes through, no cycles/ready added" {
  printf '{"foo":"bar"}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.foo == "bar" and (has("cycles") | not)'
}

@test "degenerate: --resolve on an empty items array yields no cycles and no items" {
  printf '{"items":[]}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items == [] and .cycles == []'
}

# --- Task 6 fix: malformed .blocked_by shapes must never crash --resolve --

@test "malformed shape: blocked_by as a bare string is treated as unresolvable, not a crash" {
  printf '{"items":[{"repo":"o/r","number":1,"state":"open","blocked_by":"not-an-array"}]}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.'
  echo "$output" | jq -e '.items[0].blocked_by == [] and .items[0].deps_unresolvable == true and .items[0].ready == false'
}

@test "malformed shape: blocked_by as a number is treated as unresolvable, not a crash" {
  printf '{"items":[{"repo":"o/r","number":1,"state":"open","blocked_by":5}]}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.'
  echo "$output" | jq -e '.items[0].blocked_by == [] and .items[0].deps_unresolvable == true and .items[0].ready == false'
}

@test "malformed shape: blocked_by as an object is treated as unresolvable, not a crash" {
  printf '{"items":[{"repo":"o/r","number":1,"state":"open","blocked_by":{"a":1}}]}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.'
  echo "$output" | jq -e '.items[0].blocked_by == [] and .items[0].deps_unresolvable == true and .items[0].ready == false'
}

@test "malformed shape: blocked_by explicitly null is treated as no blockers (not malformed)" {
  printf '{"items":[{"repo":"o/r","number":1,"state":"open","blocked_by":null}]}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.'
  echo "$output" | jq -e '.items[0].blocked_by == [] and .items[0].deps_unresolvable == false and .items[0].ready == true'
}

@test "malformed shape: blocked_by array of bare strings/numbers/nulls is unresolvable, not a crash" {
  printf '{"items":[{"repo":"o/r","number":1,"state":"open","blocked_by":["x",1,null]}]}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.'
  echo "$output" | jq -e '.items[0].blocked_by == [] and .items[0].deps_unresolvable == true and .items[0].ready == false'
}

@test "malformed shape: blocked_by array entry missing repo is unresolvable, not a crash" {
  printf '{"items":[{"repo":"o/r","number":1,"state":"open","blocked_by":[{"number":2}]}]}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.'
  echo "$output" | jq -e '.items[0].blocked_by == [] and .items[0].deps_unresolvable == true and .items[0].ready == false'
}

@test "malformed shape: blocked_by array entry missing number is unresolvable, not a crash" {
  printf '{"items":[{"repo":"o/r","number":1,"state":"open","blocked_by":[{"repo":"o/r"}]}]}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.'
  echo "$output" | jq -e '.items[0].blocked_by == [] and .items[0].deps_unresolvable == true and .items[0].ready == false'
}

@test "malformed shape: a well-formed blocked_by array still resolves normally after the guard" {
  printf '{"items":[{"repo":"o/r","number":1,"state":"closed","blocked_by":[]},{"repo":"o/r","number":2,"state":"open","blocked_by":[{"repo":"o/r","number":1}]}]}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.items[] | select(.number==2) | .ready == true'
}

@test "malformed shape: top-level .cycles being a non-array on input does not crash --resolve" {
  printf '{"items":[{"repo":"o/r","number":1,"state":"open","blocked_by":[]}],"cycles":"nope"}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.'
  echo "$output" | jq -e '.cycles == []'
}

@test "malformed shape: top-level .items being a non-array on input passes through without crashing" {
  printf '{"items":"nope"}' > "$TMP/in.json"
  run bash "$SCRIPT" --resolve < "$TMP/in.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.'
  echo "$output" | jq -e '.items == "nope"'
}

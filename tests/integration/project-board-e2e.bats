#!/usr/bin/env bats
# End-to-end: real board captures -> resolve -> normalize -> deps -> readiness.
#
# Uses the pinned fixtures (tests/fixtures/project-board/{p1,p2}-items.json),
# so this runs offline, deterministically, and in CI — no network, no `gh`,
# no live GitHub Projects API calls. Fixtures are converted to plan shape
# with jq rather than invoking project-board-resolve.sh's network path.
#
# Measured baselines (see task-14-report.md for reproduction):
#   p2: 80 items, 1 repo, 78/80 have dependency edges, ready=1, blocked=79,
#       unresolvable=1, cycles=0. Item #1 ("Blocked by: none.") is ready;
#       item #80 (the Phase 5.5 audit, declared in prose with no #N) is
#       deps_unresolvable and NOT ready.
#   p1: 80 items, 6 repos, 54/80 have dependency edges, ready=26, blocked=54,
#       unresolvable=0, cycles=0. 30/80 items carry no priority label
#       (normalized.priority is correctly null for them).

setup() {
  TMP="$(mktemp -d)"
  S="${BATS_TEST_DIRNAME}/../../scripts"
  FIX="${BATS_TEST_DIRNAME}/../fixtures/project-board"
}
teardown() { rm -rf "$TMP"; }

# Converts a pinned project-board-resolve.sh capture into plan shape
# (the input project-board-normalize.sh / project-board-deps.sh expect),
# without touching the network.
plan_from() {
  jq '{project:{owner:"InferWeave",kind:"org",number:0},fields:{},
       repos: [.items[].content.repository] | unique,
       items: [.items[] | {item_id:.id, repo:.content.repository, number:.content.number,
                           title:.content.title, body:(.content.body // ""),
                           state:"open", labels:(.labels // []),
                           dependencies:null, parent_issue:null}]}' "$1"
}

@test "p2: full pipeline runs end to end and emits valid JSON" {
  plan_from "$FIX/p2-items.json" > "$TMP/plan.json"
  run bash -c "bash '$S/project-board-normalize.sh' < '$TMP/plan.json' | bash '$S/project-board-deps.sh' --resolve"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.' > /dev/null
  echo "$output" > "$TMP/p2out.json"
}

@test "p2: headline counts match the measured baseline" {
  plan_from "$FIX/p2-items.json" > "$TMP/plan.json"
  run bash -c "bash '$S/project-board-normalize.sh' < '$TMP/plan.json' | bash '$S/project-board-deps.sh' --resolve"
  [ "$status" -eq 0 ]
  echo "$output" > "$TMP/p2out.json"
  [ "$(jq '.items | length' "$TMP/p2out.json")" -eq 80 ]
  [ "$(jq '.repos | length' "$TMP/p2out.json")" -eq 1 ]
  [ "$(jq '[.items[] | select((.blocked_by | length) > 0)] | length' "$TMP/p2out.json")" -eq 78 ]
  [ "$(jq '[.items[] | select(.ready)] | length' "$TMP/p2out.json")" -eq 1 ]
  [ "$(jq '[.items[] | select(.ready | not)] | length' "$TMP/p2out.json")" -eq 79 ]
  [ "$(jq '[.items[] | select(.deps_unresolvable == true)] | length' "$TMP/p2out.json")" -eq 1 ]
  [ "$(jq '.cycles | length' "$TMP/p2out.json")" -eq 0 ]
}

@test "p1: headline counts match the measured baseline" {
  plan_from "$FIX/p1-items.json" > "$TMP/plan.json"
  run bash -c "bash '$S/project-board-normalize.sh' < '$TMP/plan.json' | bash '$S/project-board-deps.sh' --resolve"
  [ "$status" -eq 0 ]
  echo "$output" > "$TMP/p1out.json"
  [ "$(jq '.items | length' "$TMP/p1out.json")" -eq 80 ]
  [ "$(jq '.repos | length' "$TMP/p1out.json")" -eq 6 ]
  [ "$(jq '[.items[] | select((.blocked_by | length) > 0)] | length' "$TMP/p1out.json")" -eq 54 ]
  [ "$(jq '[.items[] | select(.ready)] | length' "$TMP/p1out.json")" -eq 26 ]
  [ "$(jq '[.items[] | select(.ready | not)] | length' "$TMP/p1out.json")" -eq 54 ]
  [ "$(jq '[.items[] | select(.deps_unresolvable == true)] | length' "$TMP/p1out.json")" -eq 0 ]
  [ "$(jq '.cycles | length' "$TMP/p1out.json")" -eq 0 ]
}

@test "p2: item #1 is ready and item #80 (the final audit) is not" {
  # This is the ordering guarantee the whole pipeline exists to enforce: the
  # Phase 5.5 audit (#80, declared in prose with no parseable #N) must never
  # be promoted as ready before the 78 issues it audits. If this regresses,
  # autospec would implement the audit ahead of the work it is meant to
  # audit.
  plan_from "$FIX/p2-items.json" > "$TMP/plan.json"
  run bash -c "bash '$S/project-board-normalize.sh' < '$TMP/plan.json' | bash '$S/project-board-deps.sh' --resolve"
  [ "$status" -eq 0 ]
  echo "$output" > "$TMP/p2out.json"
  [ "$(jq '.items[] | select(.number == 1) | .ready' "$TMP/p2out.json")" = "true" ]
  [ "$(jq '.items[] | select(.number == 80) | .ready' "$TMP/p2out.json")" = "false" ]
  [ "$(jq '.items[] | select(.number == 80) | .deps_unresolvable' "$TMP/p2out.json")" = "true" ]
}

@test "both boards normalize priority labels into one vocabulary, nulls stay null" {
  for f in p1 p2; do
    plan_from "$FIX/$f-items.json" > "$TMP/$f-plan.json"
    run bash -c "bash '$S/project-board-normalize.sh' < '$TMP/$f-plan.json'"
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '
      [.items[].normalized.priority] as $p
      | ($p | map(select(. != null)) | unique | all(. as $v | ["critical","high","normal","low"] | index($v) != null))
      and (($p | map(select(. == null)) | length) >= 0)
    ' > /dev/null
  done
  # p1 carries exactly 30 null priorities (30/80 items have no priority label)
  # and only the p0/p1-style vocabulary observed in this fixture.
  plan_from "$FIX/p1-items.json" > "$TMP/p1-plan.json"
  run bash -c "bash '$S/project-board-normalize.sh' < '$TMP/p1-plan.json'"
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.items[].normalized.priority] | map(select(. == null)) | length')" -eq 30 ]
  # p2 carries the priority/critical-style vocabulary and zero nulls observed
  # in this fixture; both taxonomies land in the same canonical set.
  plan_from "$FIX/p2-items.json" > "$TMP/p2-plan.json"
  run bash -c "bash '$S/project-board-normalize.sh' < '$TMP/p2-plan.json'"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.items[].normalized.priority] | unique | sort == ["critical","high","normal"]' > /dev/null
}

@test "p1: 'Depends on issue #N' phrasing parses; only recognizing 'Blocked by' yields zero edges" {
  plan_from "$FIX/p1-items.json" > "$TMP/plan.json"
  run bash -c "bash '$S/project-board-normalize.sh' < '$TMP/plan.json' | bash '$S/project-board-deps.sh'"
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.items[] | select((.blocked_by | length) > 0)] | length')" -eq 54 ]

  run bash -c "AUTOSPEC_PROJECT_BOARD_DEP_MARKERS='Blocked by' bash '$S/project-board-normalize.sh' < '$TMP/plan.json' | AUTOSPEC_PROJECT_BOARD_DEP_MARKERS='Blocked by' bash '$S/project-board-deps.sh'"
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.items[] | select((.blocked_by | length) > 0)] | length')" -eq 0 ]
}

@test "p2: 'Blocked by: #N' phrasing parses and both boards report zero cycles" {
  plan_from "$FIX/p2-items.json" > "$TMP/plan.json"
  run bash -c "bash '$S/project-board-normalize.sh' < '$TMP/plan.json' | bash '$S/project-board-deps.sh' --resolve"
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '[.items[] | select((.blocked_by | length) > 0)] | length')" -eq 78 ]
  [ "$(echo "$output" | jq '.cycles | length')" -eq 0 ]
}

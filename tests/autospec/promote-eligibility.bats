setup() { TMP="$(mktemp -d)"; SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/promote-eligibility.sh"; }
teardown() { rm -rf "$TMP"; }
mkbody() { printf '%s' "$1" > "$TMP/b"; }

@test "clear single-file bug fix is eligible" {
  mkbody "fix: guard set -eu abort in loop.sh line 1250; repro: run conductor with empty backlog, observe crash. Expected: no crash."
  run bash "$SCRIPT" "$TMP/b" --labels "bug"
  echo "$output" | jq -e '.decision == "eligible"'
}

@test "lint-passing complete issue template is eligible without string intent tokens" {
  mkbody '## Goal

Resolve gap `gap-1` in `scripts/widget.sh` using captured evidence.

## Files to read first

- `scripts/widget.sh`

## Implementation scope

- Correct the reported behavior in `scripts/widget.sh`.

## Implementation outline

1. Reproduce the reported behavior.
2. Apply the scoped correction.

## Tests required

- Add a regression test for `scripts/widget.sh`.

## Dependencies

none

## Files touched

- `scripts/widget.sh`

## Acceptance criteria

- [ ] A regression test covering the reported gap passes 1 time.

## Verification

### Primary smoke test (inner loop)

```bash
git diff --check
```'
  run bash "$SCRIPT" "$TMP/b" --labels "needs-classify,gap-remediation"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.decision == "eligible" and (.reason | test("quality"; "i"))'
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

# --- structured-intent recognition (pinned to the real #1463-1467 shape) ------
# These discovery-filed issues express clear intent via ## Goal + ## Suggested AC
# checkboxes (NOT a fix:/feat: body token); the old heuristic wrongly held them
# as "ambiguous". They must route to needs-template (codex fill), staying
# fail-closed for genuinely thin/structureless bodies.

@test "structured issue with AC checkboxes routes to needs-template (not hold)" {
  mkbody "## Goal
Prevent the autonomous conductor from idling when main reports no-status pending.

## Observed Evidence
The conductor logged 'main-health pending — skipping drain' every cycle with total_count:0.

## Suggested AC
- [ ] Treat pending + total_count:0 as a distinct verdict, not an infinite wait.
- [ ] Emit an actionable health reason when Tier 1 is blocked.
- [ ] Regression tests cover no-status pending, true pending, red, green."
  run bash "$SCRIPT" "$TMP/b" --labels "needs-classify,needs-autospec-template" --title "fix: prevent conductor idle loop"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.decision == "needs-template" and (.reason|test("checkbox";"i"))'
}

@test "structured issue via intent section header (no checkboxes) routes to needs-template" {
  mkbody "## Goal
Autospec should validate that routes, assistant links, and specs agree on access policy across dashboard surfaces so nothing widens scope silently.

## Proposed Behavior
Compare declared access policy against the assistant and dashboard route tables and report disagreements."
  run bash "$SCRIPT" "$TMP/b" --labels "needs-classify" --title "gap: validate access policy"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.decision == "needs-template" and (.reason|test("section header";"i"))'
}

@test "typed title prefix alone routes an otherwise-plain body to needs-template" {
  mkbody "The autonomous conductor should compare PR checks against the base branch and report which failures are inherited CI rot versus caused by the branch, so operators are not misled by pre-existing red."
  run bash "$SCRIPT" "$TMP/b" --labels "needs-classify" --title "gap: distinguish inherited CI rot"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.decision == "needs-template" and (.reason|test("title";"i"))'
}

@test "structured signals do NOT rescue a genuinely thin body (fail-closed)" {
  mkbody "please fix"
  run bash "$SCRIPT" "$TMP/b" --labels "" --title "fix: something"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.decision == "hold"'
}

@test "long plain prose with no intent/structure/typed-title still holds (no over-widening)" {
  mkbody "There is a general feeling that the dashboard could be nicer and maybe faster and we should think about improving the overall experience at some point soon perhaps."
  run bash "$SCRIPT" "$TMP/b" --labels "" --title "some thoughts"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.decision == "hold"'
}

@test "header match is line-anchored: '## Goalkeeper' does not false-trigger needs-template" {
  mkbody "## Goalkeeper roster
Some notes about the goalkeeper roster and general standings, no actionable intent stated anywhere in this reasonably long body of prose text."
  run bash "$SCRIPT" "$TMP/b" --labels "" --title "roster notes"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.decision == "hold"'
}

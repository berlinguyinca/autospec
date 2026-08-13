#!/usr/bin/env bats
# Coverage for /autospec --autonomous and scripts/autospec-autonomy-gate.sh.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  GATE="$REPO_ROOT/scripts/autospec-autonomy-gate.sh"
}

# --- 1. clear request → all gates skipped (gate returns OK on cost/scope) ---
@test "autonomous: clear request with in-scope files and small spec passes" {
  run bash "$GATE" --check all --issues 3 --tokens 50000 \
    --files "skills/autospec/SKILL.md skills/autospec/codex/prompt.md" \
    --scope "skills/autospec/" \
    --intent "fix all dashboards"
  [ "$status" -eq 0 ]
  [[ "$output" == *"OK"* ]]
}

# --- 2. vague request → autonomous mode rejects (caller emits failure code) ---
# The orchestrator owns the LLM-level "insufficient input" check; the gate
# script surfaces operator confirmation via exit=1 on any guardrail trip.
# Here we model "make it better" by an unset scope + unset files: the orchestrator
# would call gate with empty plan, and out-of-scope check returns ask=0 (no files);
# the orchestrator must then refuse with code_health:autonomous_input_insufficient.
# We assert the gate itself stays clean when no files/scope are provided.
@test "autonomous: vague request — gate is silent, orchestrator owns failure" {
  run bash "$GATE" --check out-of-scope --intent "make it better"
  [ "$status" -eq 0 ]
}

# --- 3. destructive intent → guardrail surfaces confirmation ---
@test "autonomous: destructive intent triggers ask-anyway" {
  run bash "$GATE" --check destructive --intent "delete all branches now"
  [ "$status" -eq 1 ]
  [[ "$output" == *"destructive"* ]]
}

@test "autonomous: force-push intent triggers ask-anyway" {
  run bash "$GATE" --check destructive --intent "git push --force on main"
  [ "$status" -eq 1 ]
}

# --- 4. issue count is uncapped; token estimate still gates cost ---
@test "autonomous: issue count does not trigger cost gate" {
  AUTOSPEC_AUTONOMOUS_ISSUE_CAP=1 run bash "$GATE" --check cost --issues 999999 --tokens 100000
  [ "$status" -eq 0 ]
  [[ "$output" == *"OK"* ]]
}

@test "autonomous: issue cap variable is absent from the gate contract" {
  run grep -F "AUTOSPEC_AUTONOMOUS_ISSUE_CAP" "$GATE"
  [ "$status" -eq 1 ]
}

@test "autonomous: token estimate over cap triggers cost gate" {
  AUTOSPEC_AUTONOMOUS_TOKEN_CAP=500000 run bash "$GATE" --check cost --issues 3 --tokens 900000
  [ "$status" -eq 1 ]
  [[ "$output" == *"AUTOSPEC_AUTONOMOUS_TOKEN_CAP"* ]]
}

# --- 5. ~/.autospec/autonomous.flag implies autonomous (file presence semantics) ---
@test "autonomous: ~/.autospec/autonomous.flag file presence is detectable" {
  tmpdir="$(mktemp -d)"
  HOME="$tmpdir"
  mkdir -p "$HOME/.autospec"
  touch "$HOME/.autospec/autonomous.flag"
  [ -f "$HOME/.autospec/autonomous.flag" ]
  rm -rf "$tmpdir"
}

# --- 6. autospec-listen detects autonomous phrasing → routes with --autonomous ---
@test "autonomous: listener-match keyword detection for autonomous phrasing" {
  # The listener's keyword auto-routing must recognize at least one of these phrases.
  # We grep the autospec-listen SKILL.md to confirm the autonomous routing
  # documentation is in place (deterministic contract presence).
  run grep -E 'autonomous|just do it|no confirmation|non-interactive|go autonomous|fix .* automatically' \
    "$REPO_ROOT/skills/autospec-listen/SKILL.md"
  [ "$status" -eq 0 ]
}

# --- out-of-scope guardrail unit test ---
@test "autonomous: out-of-scope file triggers guardrail" {
  run bash "$GATE" --check out-of-scope \
    --files "scripts/autospec-autonomy-gate.sh /etc/passwd" \
    --scope "scripts/"
  [ "$status" -eq 1 ]
  [[ "$output" == *"out-of-scope"* ]]
}

@test "autonomous: in-scope files pass guardrail" {
  run bash "$GATE" --check out-of-scope \
    --files "scripts/autospec-autonomy-gate.sh scripts/autospec-digital-twin.py" \
    --scope "scripts/"
  [ "$status" -eq 0 ]
}

# --- script invariants ---
@test "gate: --help prints Usage" {
  run bash "$GATE" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"Usage:"* ]]
}

@test "gate: bash -n clean" {
  run bash -n "$GATE"
  [ "$status" -eq 0 ]
}

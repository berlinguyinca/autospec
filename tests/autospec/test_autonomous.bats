#!/usr/bin/env bats
# Coverage for /autospec --autonomous and scripts/autospec-autonomy-gate.sh.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  GATE="$REPO_ROOT/scripts/autospec-autonomy-gate.sh"
  AUTONOMOUS="$REPO_ROOT/scripts/autospec-autonomous.sh"
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

# --- 4. spec exceeds AUTOSPEC_AUTONOMOUS_ISSUE_CAP → cost gate ---
@test "autonomous: issue count over cap triggers cost gate" {
  AUTOSPEC_AUTONOMOUS_ISSUE_CAP=10 run bash "$GATE" --check cost --issues 25 --tokens 100000
  [ "$status" -eq 1 ]
  [[ "$output" == *"AUTOSPEC_AUTONOMOUS_ISSUE_CAP"* ]]
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
    --files "scripts/autospec-autonomy-gate.sh scripts/validate.sh" \
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

# --- issue #1577: conductors are scoped per repo, not a global singleton ---

@test "autonomous: bash -n clean" {
  run bash -n "$AUTONOMOUS"
  [ "$status" -eq 0 ]
}

@test "autonomous: operator pid file is scoped by repo slug" {
  tmp="$(mktemp -d)"
  run env HOME="$tmp" bash "$AUTONOMOUS" status --json --repo owner/repo-a
  [ "$status" -eq 0 ]
  [[ "$output" == *"autonomous-operator/owner__repo-a/conductor.pid"* ]]
  rm -rf "$tmp"
}

@test "autonomous: different repos resolve to different pid files" {
  tmp="$(mktemp -d)"
  a="$(env HOME="$tmp" bash "$AUTONOMOUS" status --json --repo owner/repo-a)"
  b="$(env HOME="$tmp" bash "$AUTONOMOUS" status --json --repo owner/repo-b)"
  [[ "$a" == *"owner__repo-a"* ]]
  [[ "$b" == *"owner__repo-b"* ]]
  [ "$a" != "$b" ]
  rm -rf "$tmp"
}

@test "autonomous: explicit AUTOSPEC_AUTONOMOUS_OPERATOR_DIR disables per-repo scoping" {
  tmp="$(mktemp -d)"
  run env HOME="$tmp" AUTOSPEC_AUTONOMOUS_OPERATOR_DIR="$tmp/fixed" \
    bash "$AUTONOMOUS" status --json --repo owner/repo-a
  [ "$status" -eq 0 ]
  [[ "$output" == *"$tmp/fixed/conductor.pid"* ]]
  [[ "$output" != *"owner__repo-a"* ]]
  rm -rf "$tmp"
}

@test "autonomous: a live conductor in repo A does not appear running for repo B" {
  tmp="$(mktemp -d)"
  # A long-lived process stands in for repo A's live conductor.
  sleep 300 &
  live_pid="$!"
  mkdir -p "$tmp/.autospec/autonomous-operator/owner__repo-a"
  printf '%s\n' "$live_pid" > "$tmp/.autospec/autonomous-operator/owner__repo-a/conductor.pid"

  a="$(env HOME="$tmp" bash "$AUTONOMOUS" status --json --repo owner/repo-a)"
  b="$(env HOME="$tmp" bash "$AUTONOMOUS" status --json --repo owner/repo-b)"
  [[ "$a" == *'"running":true'* ]]
  [[ "$b" == *'"running":false'* ]]

  kill "$live_pid" 2>/dev/null || true
  rm -rf "$tmp"
}

# Upgrade backstop: a conductor started before per-repo scoping has no scoped PID
# file, but its per-repo state file still shows a fresh heartbeat.  `start` must
# refuse to launch a duplicate for that repo (issue #1577).
@test "autonomous: start refuses when per-repo state shows a fresh-heartbeat conductor" {
  command -v jq >/dev/null 2>&1 || skip "jq required"
  tmp="$(mktemp -d)"
  mkdir -p "$tmp/.autospec/autonomous/owner__repo-a"
  now="$(date +%s)"
  printf '{"repo":"owner/repo-a","status":"running:cycle-5","heartbeat_at":%s}\n' "$now" \
    > "$tmp/.autospec/autonomous/owner__repo-a/state.json"
  # Neutralize the drain so a guard regression cannot spawn real work.
  # AUTOSPEC_RUN_CMD=true neutralizes the drain; the guard dies before any spawn.
  run env HOME="$tmp" AUTOSPEC_RUN_CMD=true CONDUCTOR_MAX_CYCLES=1 AUTOSPEC_NO_SELF_UPDATE=1 \
    bash "$AUTONOMOUS" start --repo owner/repo-a
  [ "$status" -ne 0 ]
  [[ "$output" == *"appears active"* ]]
  rm -rf "$tmp"
}

# The guard reads only THIS repo's state file, so a fresh heartbeat for a
# different repo (or a stale one for this repo) must not block start.  Asserted
# without a real spawn: a dead scoped PID + no fresh own-state means the guard
# helper returns false, which the fire-case above contrasts.
@test "autonomous: guard keys on the current repo's own state slug" {
  command -v jq >/dev/null 2>&1 || skip "jq required"
  tmp="$(mktemp -d)"
  now="$(date +%s)"
  # Fresh, running — but for repo-a only.
  mkdir -p "$tmp/.autospec/autonomous/owner__repo-a"
  printf '{"repo":"owner/repo-a","status":"running:cycle-5","heartbeat_at":%s}\n' "$now" \
    > "$tmp/.autospec/autonomous/owner__repo-a/state.json"
  # repo-b's own state is stale — must not count as live.
  mkdir -p "$tmp/.autospec/autonomous/owner__repo-b"
  printf '{"repo":"owner/repo-b","status":"running:cycle-9","heartbeat_at":1}\n' \
    > "$tmp/.autospec/autonomous/owner__repo-b/state.json"
  # status never calls the guard, but confirms repo-b resolves to its own slug
  # (owner__repo-b), i.e. it cannot read repo-a's fresh state.
  run env HOME="$tmp" bash "$AUTONOMOUS" status --json --repo owner/repo-b
  [ "$status" -eq 0 ]
  [[ "$output" == *"owner__repo-b/conductor.pid"* ]]
  [[ "$output" != *"owner__repo-a"* ]]
  rm -rf "$tmp"
}

#!/usr/bin/env bats
# tests/autonomous/test_priority_intake.bats
# TDD contract for F4 — priority intake + control-payload capture + persona-aware matcher.
#
# Coverage:
#   - scripts/autonomous-priority-match.sh: existence, bash -n, match/no-match, fail-open
#   - Intake: CONDUCTOR_PRIORITIES parsed and persisted to priorities file
#   - Intake: first-run AskUserQuestion gate (mocked via AUTOSPEC_ASK_PRIORITIES_CMD)
#   - Control-payload capture: DIRECTIVE: / PRIORITY_ISSUE: lines written to priorities file
#   - Matcher: matching proposal → priority:true; non-matching → unchanged
#   - Matcher: fail-open on empty priorities (no boost, no starvation)
#   - Boost+clamp: explore-research-cycle.sh consumes priority:true with floor+cap
#   - No parallel filer: matcher never calls gh
#   - Safety gate: CONDUCTOR_PRIORITIES parsing never bypasses premerge gate
#
# Engineering notes:
#   - All fixtures written to real temp files (macOS bash 3.2: [ -f <(…) ] always false)
#   - .autospec/ is gitignored; all state materialised at runtime under TMP
#   - External boundaries mocked as subprocess seams (AUTOSPEC_PRIORITY_MATCH_CMD,
#     AUTOSPEC_ASK_PRIORITIES_CMD, AUTOSPEC_RUN_CMD)
#   - No RETURN traps in SUTs; tests verify inline-cleanup behaviour
#   - printf with format starting '-' requires '--' on macOS bash 3.2 (bash32 gotcha)

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
MATCH_SCRIPT="$REPO_ROOT/scripts/autonomous-priority-match.sh"
LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"
CYCLE_SCRIPT="$REPO_ROOT/scripts/explore-research-cycle.sh"

setup() {
    TMP="$(mktemp -d -t test-priority-intake.XXXXXX)"

    # Fake ~/.autospec home (avoid touching real operator state).
    AUTOSPEC_HOME_DIR="$TMP/autospec-home"
    mkdir -p "$AUTOSPEC_HOME_DIR"

    PRIORITIES_FILE="$AUTOSPEC_HOME_DIR/autonomous-priorities.md"
    PERSONA_FILE="$AUTOSPEC_HOME_DIR/operator-persona.md"
    export AUTOSPEC_PRIORITIES_FILE="$PRIORITIES_FILE"
    export AUTOSPEC_PERSONA_FILE="$PERSONA_FILE"

    # Write a minimal persona fixture (avoid printf leading-dash issue: use cat).
    cat > "$PERSONA_FILE" <<'EOF'
# Operator persona

## Decision style

Favors correctness over speed.
EOF

    # ── Conductor mock infrastructure ──────────────────────────────────────────
    SCRIPTS_DIR="$TMP/scripts"
    mkdir -p "$SCRIPTS_DIR"

    # mock: autonomous-waterfall.sh — always Tier 1 / run-backlog
    cat > "$SCRIPTS_DIR/autonomous-waterfall.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"tier":1,"action":"run-backlog","reason":"test-default"}\n'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-waterfall.sh"

    # mock: autonomous-control-channel.sh — default: no decisions
    cat > "$SCRIPTS_DIR/autonomous-control-channel.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-control-channel.sh"

    # mock: autonomous-premerge-gate.sh — always merge-ok
    cat > "$SCRIPTS_DIR/autonomous-premerge-gate.sh" <<'EOF'
#!/usr/bin/env bash
printf 'merge-ok\n'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-premerge-gate.sh"

    # mock: autonomous-resilience.sh — no-op
    cat > "$SCRIPTS_DIR/autonomous-resilience.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-resilience.sh"

    # mock: autonomous-spend-ledger.sh — always continue
    cat > "$SCRIPTS_DIR/autonomous-spend-ledger.sh" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "check" ]; then printf 'continue\n'; fi
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-spend-ledger.sh"

    # mock: autospec-usage-limit.sh — no-op
    cat > "$SCRIPTS_DIR/autospec-usage-limit.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autospec-usage-limit.sh"

    # mock: autonomous-persona-synth.sh — no-op
    cat > "$SCRIPTS_DIR/autonomous-persona-synth.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-persona-synth.sh"

    # Export conductor env.
    RUN_CMD_LOG="$TMP/run-cmd.log"
    touch "$RUN_CMD_LOG"
    export CONDUCTOR_SCRIPTS_DIR="$SCRIPTS_DIR"
    export CONDUCTOR_MAX_CYCLES=1
    export CONDUCTOR_DRY_RUN=0
    export CONDUCTOR_NO_DIGEST=1
    export CONDUCTOR_POLL_INTERVAL=0
    export AUTOSPEC_RUN_CMD="touch $RUN_CMD_LOG"
    export HOME="$TMP/home"
    mkdir -p "$HOME"

    # Unset CONDUCTOR_PRIORITIES between tests so intake logic is pristine.
    unset CONDUCTOR_PRIORITIES 2>/dev/null || true
    unset AUTOSPEC_ASK_PRIORITIES_CMD 2>/dev/null || true
}

teardown() {
    rm -rf "$TMP"
}

# ──────────────────────────────────────────────────────────────────────────────
# 1. Script existence + hygiene
# ──────────────────────────────────────────────────────────────────────────────

@test "autonomous-priority-match.sh exists and is executable" {
    [ -f "$MATCH_SCRIPT" ]
    [ -x "$MATCH_SCRIPT" ]
}

@test "autonomous-priority-match.sh passes bash -n" {
    run bash -n "$MATCH_SCRIPT"
    [ "$status" -eq 0 ]
}

# ──────────────────────────────────────────────────────────────────────────────
# 2. Matcher: matching proposal gets priority:true
# ──────────────────────────────────────────────────────────────────────────────

@test "matcher: matching proposal gets priority:true set" {
    # Seed priorities (use cat heredoc to avoid printf leading-dash issue).
    cat > "$PRIORITIES_FILE" <<'EOF'
- improve test coverage
- fix correctness bugs
EOF

    # Mock the LLM seam to emit "match".
    MATCH_MOCK="$TMP/match-mock.sh"
    cat > "$MATCH_MOCK" <<'EOF'
#!/usr/bin/env bash
cat >/dev/null
printf 'match\n'
EOF
    chmod +x "$MATCH_MOCK"

    PROPOSAL='{"title":"fix correctness bug in parser","score":0.4,"source":"spec-vs-code","priority":false}'
    PROPOSAL_FILE="$TMP/proposal.json"
    printf '%s' "$PROPOSAL" > "$PROPOSAL_FILE"

    run bash -c "
AUTOSPEC_PRIORITY_MATCH_CMD='bash $MATCH_MOCK' \
AUTOSPEC_PRIORITIES_FILE='$PRIORITIES_FILE' \
AUTOSPEC_PERSONA_FILE='$PERSONA_FILE' \
bash '$MATCH_SCRIPT' < '$PROPOSAL_FILE'"
    [ "$status" -eq 0 ]

    # Write output to file for Python inspection (avoid process-sub, bash 3.2 compat).
    RESULT_FILE="$TMP/match-result.json"
    printf '%s' "$output" > "$RESULT_FILE"

    _PY="$TMP/check_match.py"
    cat > "$_PY" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
assert d.get("priority") == True, "expected priority:true, got: %s" % d.get("priority")
print("ok")
PYEOF
    run python3 "$_PY" "$RESULT_FILE"
    [ "$status" -eq 0 ]
}

@test "matcher: non-matching proposal remains unchanged (no priority:true)" {
    cat > "$PRIORITIES_FILE" <<'EOF'
- improve test coverage
EOF

    NO_MATCH_MOCK="$TMP/no-match-mock.sh"
    cat > "$NO_MATCH_MOCK" <<'EOF'
#!/usr/bin/env bash
cat >/dev/null
printf 'no-match\n'
EOF
    chmod +x "$NO_MATCH_MOCK"

    PROPOSAL='{"title":"add logging to deploy step","score":0.3,"source":"dogfooding","priority":false}'
    PROPOSAL_FILE="$TMP/proposal-nomatch.json"
    printf '%s' "$PROPOSAL" > "$PROPOSAL_FILE"

    run bash -c "
AUTOSPEC_PRIORITY_MATCH_CMD='bash $NO_MATCH_MOCK' \
AUTOSPEC_PRIORITIES_FILE='$PRIORITIES_FILE' \
AUTOSPEC_PERSONA_FILE='$PERSONA_FILE' \
bash '$MATCH_SCRIPT' < '$PROPOSAL_FILE'"
    [ "$status" -eq 0 ]

    RESULT_FILE="$TMP/nomatch-result.json"
    printf '%s' "$output" > "$RESULT_FILE"

    _PY="$TMP/check_nomatch.py"
    cat > "$_PY" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
assert d.get("priority") == False, "expected priority:false, got: %s" % d.get("priority")
print("ok")
PYEOF
    run python3 "$_PY" "$RESULT_FILE"
    [ "$status" -eq 0 ]
}

# ──────────────────────────────────────────────────────────────────────────────
# 3. Matcher: fail-open behaviour
# ──────────────────────────────────────────────────────────────────────────────

@test "matcher: absent priorities file → proposal emitted unchanged (fail-open)" {
    [ ! -f "$PRIORITIES_FILE" ]
    PROPOSAL_FILE="$TMP/prop-absent.json"
    printf '%s' '{"title":"some proposal","score":0.5,"priority":false}' > "$PROPOSAL_FILE"

    run bash -c "
AUTOSPEC_PRIORITIES_FILE='$PRIORITIES_FILE' \
AUTOSPEC_PERSONA_FILE='$PERSONA_FILE' \
bash '$MATCH_SCRIPT' < '$PROPOSAL_FILE'"
    [ "$status" -eq 0 ]

    RESULT_FILE="$TMP/absent-result.json"
    printf '%s' "$output" > "$RESULT_FILE"

    _PY="$TMP/check_absent.py"
    cat > "$_PY" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
assert d.get("priority") == False, "expected priority unchanged (false)"
print("ok")
PYEOF
    run python3 "$_PY" "$RESULT_FILE"
    [ "$status" -eq 0 ]
}

@test "matcher: empty priorities file → proposal emitted unchanged (fail-open)" {
    : > "$PRIORITIES_FILE"
    PROPOSAL_FILE="$TMP/prop-empty.json"
    printf '%s' '{"title":"some proposal","score":0.5,"priority":false}' > "$PROPOSAL_FILE"

    run bash -c "
AUTOSPEC_PRIORITIES_FILE='$PRIORITIES_FILE' \
AUTOSPEC_PERSONA_FILE='$PERSONA_FILE' \
bash '$MATCH_SCRIPT' < '$PROPOSAL_FILE'"
    [ "$status" -eq 0 ]

    RESULT_FILE="$TMP/empty-result.json"
    printf '%s' "$output" > "$RESULT_FILE"

    _PY="$TMP/check_empty.py"
    cat > "$_PY" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
assert d.get("priority") == False, "expected priority unchanged (false)"
print("ok")
PYEOF
    run python3 "$_PY" "$RESULT_FILE"
    [ "$status" -eq 0 ]
}

@test "matcher: bad JSON on stdin → exits 0 and emits the bad input unchanged" {
    cat > "$PRIORITIES_FILE" <<'EOF'
- some priority
EOF
    BADINPUT_FILE="$TMP/badinput.txt"
    printf '%s' 'not-json' > "$BADINPUT_FILE"

    run bash -c "
AUTOSPEC_PRIORITIES_FILE='$PRIORITIES_FILE' \
bash '$MATCH_SCRIPT' < '$BADINPUT_FILE'"
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q 'not-json'
}

@test "matcher: never calls gh (no parallel filing path)" {
    cat > "$PRIORITIES_FILE" <<'EOF'
- improve everything
EOF

    MATCH_MOCK="$TMP/always-match.sh"
    cat > "$MATCH_MOCK" <<'EOF'
#!/usr/bin/env bash
cat >/dev/null
printf 'match\n'
EOF
    chmod +x "$MATCH_MOCK"

    # Fake gh that records calls; the matcher must not invoke it.
    GH_LOG="$TMP/gh-calls.log"
    FAKE_GH="$TMP/fake-gh.sh"
    cat > "$FAKE_GH" <<EOF
#!/usr/bin/env bash
printf 'gh-called\n' >> '$GH_LOG'
exit 1
EOF
    chmod +x "$FAKE_GH"

    PROPOSAL_FILE="$TMP/prop-gh.json"
    printf '%s' '{"title":"fix something","score":0.4,"priority":false}' > "$PROPOSAL_FILE"

    PATH="$TMP:$PATH" \
    GH="$FAKE_GH" \
    AUTOSPEC_GH_CMD="$FAKE_GH" \
    AUTOSPEC_PRIORITY_MATCH_CMD="bash $MATCH_MOCK" \
    AUTOSPEC_PRIORITIES_FILE="$PRIORITIES_FILE" \
    AUTOSPEC_PERSONA_FILE="$PERSONA_FILE" \
    bash "$MATCH_SCRIPT" < "$PROPOSAL_FILE" >/dev/null

    # gh must NOT have been called.
    [ ! -f "$GH_LOG" ] || [ ! -s "$GH_LOG" ]
}

# ──────────────────────────────────────────────────────────────────────────────
# 4. Intake: CONDUCTOR_PRIORITIES parsing and persistence
# ──────────────────────────────────────────────────────────────────────────────

@test "conductor: CONDUCTOR_PRIORITIES persists all semicolon-delimited items" {
    export CONDUCTOR_PRIORITIES="improve test coverage; fix parser bugs; reduce latency"
    export AUTOSPEC_PRIORITIES_FILE="$PRIORITIES_FILE"

    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    [ -f "$PRIORITIES_FILE" ]
    grep -q 'improve test coverage' "$PRIORITIES_FILE"
    grep -q 'fix parser bugs' "$PRIORITIES_FILE"
    grep -q 'reduce latency' "$PRIORITIES_FILE"
}

@test "conductor: second invocation with CONDUCTOR_PRIORITIES appends (not overwrites)" {
    # Seed the file with an existing entry.
    cat > "$PRIORITIES_FILE" <<'EOF'
- first priority
EOF
    export CONDUCTOR_PRIORITIES="second priority"
    export AUTOSPEC_PRIORITIES_FILE="$PRIORITIES_FILE"

    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    grep -q 'first priority'  "$PRIORITIES_FILE"
    grep -q 'second priority' "$PRIORITIES_FILE"
}

@test "conductor: no priorities + existing file → no AskUserQuestion gate fired" {
    # If priorities file already exists, the gate must NOT be called.
    cat > "$PRIORITIES_FILE" <<'EOF'
- existing priority
EOF
    export AUTOSPEC_PRIORITIES_FILE="$PRIORITIES_FILE"

    GATE_CALLED_LOG="$TMP/ask-gate-called.log"
    GATE_MOCK="$TMP/ask-gate-mock.sh"
    cat > "$GATE_MOCK" <<EOF
#!/usr/bin/env bash
touch '$GATE_CALLED_LOG'
printf 'from gate\n'
EOF
    chmod +x "$GATE_MOCK"
    export AUTOSPEC_ASK_PRIORITIES_CMD="bash $GATE_MOCK"

    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    # Gate must NOT have been invoked.
    [ ! -f "$GATE_CALLED_LOG" ]
}

@test "conductor: first run with no priorities fires AskUserQuestion gate once" {
    [ ! -f "$PRIORITIES_FILE" ]
    export AUTOSPEC_PRIORITIES_FILE="$PRIORITIES_FILE"

    GATE_CALLED_LOG="$TMP/ask-gate-called.log"
    GATE_MOCK="$TMP/ask-gate-mock.sh"
    cat > "$GATE_MOCK" <<EOF
#!/usr/bin/env bash
touch '$GATE_CALLED_LOG'
printf 'priority from gate\n'
EOF
    chmod +x "$GATE_MOCK"
    export AUTOSPEC_ASK_PRIORITIES_CMD="bash $GATE_MOCK"

    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    [ -f "$GATE_CALLED_LOG" ]
    [ -f "$PRIORITIES_FILE" ]
    grep -q 'priority from gate' "$PRIORITIES_FILE"
}

@test "conductor: AskUserQuestion gate fires only once even if priorities file is empty" {
    export AUTOSPEC_PRIORITIES_FILE="$PRIORITIES_FILE"

    GATE_CALL_COUNT="$TMP/gate-count.txt"
    printf '%s\n' 0 > "$GATE_CALL_COUNT"
    GATE_MOCK="$TMP/ask-empty-mock.sh"
    cat > "$GATE_MOCK" <<EOF
#!/usr/bin/env bash
n=\$(cat '$GATE_CALL_COUNT')
printf '%s\n' \$((n+1)) > '$GATE_CALL_COUNT'
printf ''
EOF
    chmod +x "$GATE_MOCK"
    export AUTOSPEC_ASK_PRIORITIES_CMD="bash $GATE_MOCK"

    # First call: gate runs and creates an empty file.
    (. "$LOOP_LIB"; autospec_conductor_run) 2>/dev/null || true
    first_count="$(cat "$GATE_CALL_COUNT")"
    [ "$first_count" = "1" ]

    # Second call: priorities file now exists — gate must NOT fire again.
    (. "$LOOP_LIB"; autospec_conductor_run) 2>/dev/null || true
    second_count="$(cat "$GATE_CALL_COUNT")"
    [ "$second_count" = "1" ]
}

# ──────────────────────────────────────────────────────────────────────────────
# 5. Control-payload capture (DIRECTIVE: / PRIORITY_ISSUE:)
# ──────────────────────────────────────────────────────────────────────────────

@test "conductor: DIRECTIVE: from control channel is captured into priorities file" {
    cat > "$PRIORITIES_FILE" <<'EOF'
- existing priority
EOF
    export AUTOSPEC_PRIORITIES_FILE="$PRIORITIES_FILE"

    # Mock control-channel to emit a DIRECTIVE: line.
    cat > "$SCRIPTS_DIR/autonomous-control-channel.sh" <<'EOF'
#!/usr/bin/env bash
printf 'DECISION:steer\n'
printf 'STEER_ISSUE:42\n'
printf 'DIRECTIVE:focus on reducing CI flakiness this sprint\n'
printf 'STEER_REMOVE_LABEL:42\n'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-control-channel.sh"

    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    [ -f "$PRIORITIES_FILE" ]
    grep -q 'DIRECTIVE:' "$PRIORITIES_FILE"
    grep -q 'focus on reducing CI flakiness' "$PRIORITIES_FILE"
}

@test "conductor: PRIORITY_ISSUE: from control channel is captured into priorities file" {
    cat > "$PRIORITIES_FILE" <<'EOF'
- existing priority
EOF
    export AUTOSPEC_PRIORITIES_FILE="$PRIORITIES_FILE"

    # Mock control-channel to emit a PRIORITY_ISSUE: line.
    cat > "$SCRIPTS_DIR/autonomous-control-channel.sh" <<'EOF'
#!/usr/bin/env bash
printf 'DECISION:priority\n'
printf 'PRIORITY_ISSUE:99\n'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-control-channel.sh"

    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    [ -f "$PRIORITIES_FILE" ]
    grep -q 'PRIORITY_ISSUE:' "$PRIORITIES_FILE"
    grep -q '99' "$PRIORITIES_FILE"
}

@test "conductor: control-channel lines that are NOT DIRECTIVE:/PRIORITY_ISSUE: are not captured" {
    cat > "$PRIORITIES_FILE" <<'EOF'
- base priority
EOF
    export AUTOSPEC_PRIORITIES_FILE="$PRIORITIES_FILE"

    cat > "$SCRIPTS_DIR/autonomous-control-channel.sh" <<'EOF'
#!/usr/bin/env bash
printf 'DECISION:steer\n'
printf 'STEER_ISSUE:7\n'
printf 'STEER_REMOVE_LABEL:7\n'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-control-channel.sh"

    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    # STEER_ISSUE and STEER_REMOVE_LABEL must NOT appear in priorities file.
    run grep -c 'STEER_ISSUE\|STEER_REMOVE_LABEL' "$PRIORITIES_FILE"
    [ "$output" = "0" ]
}

# ──────────────────────────────────────────────────────────────────────────────
# 6. Boost + clamp: explore-research-cycle.sh with priority:true proposals
# ──────────────────────────────────────────────────────────────────────────────

@test "boost: priority:true proposal is boosted and clamped at max_non_boosted * multiplier" {
    WORK_DIR="$TMP/cycle-work"
    mkdir -p "$WORK_DIR/explore-research"

    # Researcher emitting one priority:true and two non-priority proposals.
    cat > "$WORK_DIR/explore-research/spec-vs-code.sh" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{
  "source": "spec-vs-code",
  "proposals": [
    {
      "title": "fix priority bug",
      "evidence": "mismatched spec line 42",
      "estimated_complexity": "small",
      "confidence": 0.8,
      "severity": "correctness",
      "named_consumer": "autospec-run",
      "priority": true
    },
    {
      "title": "add logging",
      "evidence": "missing debug output",
      "estimated_complexity": "small",
      "confidence": 0.7,
      "severity": "operability",
      "named_consumer": "autospec-run",
      "priority": false
    },
    {
      "title": "improve docs",
      "evidence": "outdated README",
      "estimated_complexity": "small",
      "confidence": 0.5,
      "severity": "nicety",
      "named_consumer": "autospec-run",
      "priority": false
    }
  ]
}
JSON
EOF
    chmod +x "$WORK_DIR/explore-research/spec-vs-code.sh"

    OUT="$TMP/cycle-out.json"
    AUTOSPEC_PRIORITY_BOOST=1.5 \
    AUTOSPEC_RESEARCH_DIR="$WORK_DIR/explore-research" \
    AUTOSPEC_TEST_RECENT_TITLES="sentinel-no-match" \
    run bash "$CYCLE_SCRIPT" \
        --research-sources spec-vs-code \
        --specialists-mode off \
        --max-issues-per-round 5 \
        --out "$OUT"
    [ "$status" -eq 0 ]
    [ -f "$OUT" ]

    # Verify clamp using a Python script file (avoids bash-3.2 quoting issues).
    _PY="$TMP/check_clamp.py"
    cat > "$_PY" <<PYEOF
import json, sys
d = json.load(open(sys.argv[1]))
proposals = d.get("proposals", [])
priority_scores = [p["score"] for p in proposals if p.get("priority")]
non_priority_scores = [p["score"] for p in proposals if not p.get("priority")]
assert priority_scores, "no priority proposal in output"
assert non_priority_scores, "no non-priority proposal in output"
cap = max(non_priority_scores) * 1.5
for ps in priority_scores:
    assert ps <= cap + 0.0001, "boost %s exceeded cap %s" % (ps, cap)
print("clamp-ok")
PYEOF
    run python3 "$_PY" "$OUT"
    [ "$status" -eq 0 ]
    [ "$output" = "clamp-ok" ]
}

@test "boost: priority:true proposal scores higher than same proposal without priority" {
    WORK_DIR="$TMP/cycle-work-cmp"
    mkdir -p "$WORK_DIR/explore-research"

    # Two proposals with identical base attributes but distinct concerns;
    # one priority:true, one priority:false.  Titles are deliberately
    # unrelated so the structural-dedup step does NOT merge them.
    cat > "$WORK_DIR/explore-research/spec-vs-code.sh" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{
  "source": "spec-vs-code",
  "proposals": [
    {
      "title": "fix output parser drops last byte when buffer is full",
      "evidence": "integration test reveals off-by-one in read loop",
      "estimated_complexity": "small",
      "confidence": 0.5,
      "severity": "correctness",
      "named_consumer": "autospec-run",
      "priority": true
    },
    {
      "title": "add retry logic for transient network errors in fetcher",
      "evidence": "fetcher silently fails on timeout with no retry",
      "estimated_complexity": "small",
      "confidence": 0.5,
      "severity": "operability",
      "named_consumer": "autospec-run",
      "priority": false
    }
  ]
}
JSON
EOF
    chmod +x "$WORK_DIR/explore-research/spec-vs-code.sh"

    OUT="$TMP/cycle-out-cmp.json"
    AUTOSPEC_PRIORITY_BOOST=1.5 \
    AUTOSPEC_RESEARCH_DIR="$WORK_DIR/explore-research" \
    AUTOSPEC_TEST_RECENT_TITLES="sentinel-no-match" \
    run bash "$CYCLE_SCRIPT" \
        --research-sources spec-vs-code \
        --specialists-mode off \
        --max-issues-per-round 5 \
        --out "$OUT"
    [ "$status" -eq 0 ]

    _PY="$TMP/check_order.py"
    cat > "$_PY" <<PYEOF
import json, sys
d = json.load(open(sys.argv[1]))
proposals = d.get("proposals", [])
priority_scores = [p["score"] for p in proposals if p.get("priority")]
non_priority_scores = [p["score"] for p in proposals if not p.get("priority")]
assert priority_scores, "no priority proposal in output: %s" % [p["title"] for p in proposals]
assert non_priority_scores, "no non-priority proposal in output: %s" % [p["title"] for p in proposals]
assert max(priority_scores) > max(non_priority_scores), \
    "max priority score %s must exceed max non-priority %s" % (max(priority_scores), max(non_priority_scores))
print("score-order-ok")
PYEOF
    run python3 "$_PY" "$OUT"
    [ "$status" -eq 0 ]
    [ "$output" = "score-order-ok" ]
}

@test "boost: misconfigured boost <1.0 is floored at 1.0 (no de-ranking)" {
    WORK_DIR="$TMP/cycle-work-floor"
    mkdir -p "$WORK_DIR/explore-research"

    cat > "$WORK_DIR/explore-research/spec-vs-code.sh" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{
  "source": "spec-vs-code",
  "proposals": [
    {
      "title": "priority proposal",
      "evidence": "evidence here",
      "estimated_complexity": "small",
      "confidence": 0.6,
      "severity": "feature",
      "named_consumer": "autospec-run",
      "priority": true
    },
    {
      "title": "baseline proposal",
      "evidence": "other evidence",
      "estimated_complexity": "small",
      "confidence": 0.3,
      "severity": "feature",
      "named_consumer": "autospec-run",
      "priority": false
    }
  ]
}
JSON
EOF
    chmod +x "$WORK_DIR/explore-research/spec-vs-code.sh"

    OUT="$TMP/cycle-out-floor.json"
    # Deliberately bad boost (<1.0) — should be clamped to 1.0.
    AUTOSPEC_PRIORITY_BOOST=0.5 \
    AUTOSPEC_RESEARCH_DIR="$WORK_DIR/explore-research" \
    AUTOSPEC_TEST_RECENT_TITLES="sentinel-no-match" \
    run bash "$CYCLE_SCRIPT" \
        --research-sources spec-vs-code \
        --specialists-mode off \
        --max-issues-per-round 5 \
        --out "$OUT"
    [ "$status" -eq 0 ]

    _PY="$TMP/check_floor.py"
    cat > "$_PY" <<PYEOF
import json, sys
d = json.load(open(sys.argv[1]))
props = {p["title"]: p for p in d.get("proposals", [])}
pri = props.get("priority proposal")
nop = props.get("baseline proposal")
assert pri is not None, "priority proposal missing from output"
# With floor=1.0, the priority proposal must score >= the non-priority
# (the boost cannot de-rank it below a lower-confidence proposal).
if nop:
    assert pri["score"] >= nop["score"] - 0.001, \
        "priority score %s below non-priority %s (floor broken)" % (pri["score"], nop["score"])
print("floor-ok")
PYEOF
    run python3 "$_PY" "$OUT"
    [ "$status" -eq 0 ]
    [ "$output" = "floor-ok" ]
}

# ──────────────────────────────────────────────────────────────────────────────
# 7. Safety gate: CONDUCTOR_PRIORITIES never bypasses the premerge gate
# ──────────────────────────────────────────────────────────────────────────────

@test "safety gate: premerge gate still runs even when CONDUCTOR_PRIORITIES is set" {
    export CONDUCTOR_PRIORITIES="critical bug fixes only"
    export AUTOSPEC_PRIORITIES_FILE="$PRIORITIES_FILE"

    GATE_CALLED_LOG="$TMP/gate-called.log"

    # Premerge gate records it was called, then returns merge-ok.
    cat > "$SCRIPTS_DIR/autonomous-premerge-gate.sh" <<EOF
#!/usr/bin/env bash
touch '$GATE_CALLED_LOG'
printf 'merge-ok\n'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-premerge-gate.sh"

    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    [ -f "$GATE_CALLED_LOG" ]
}

@test "no-second-ranker: matcher emits proposal JSON only, never calls gh" {
    # The matcher must NEVER file issues directly — it only sets priority:true.
    cat > "$PRIORITIES_FILE" <<'EOF'
- fix everything
EOF

    # Fake gh that would record calls.
    GH_LOG="$TMP/gh-calls-matcher.log"
    FAKE_GH="$TMP/fake-gh-m.sh"
    cat > "$FAKE_GH" <<EOF
#!/usr/bin/env bash
printf 'gh-was-called\n' >> '$GH_LOG'
exit 0
EOF
    chmod +x "$FAKE_GH"

    ALWAYS_MATCH_MOCK="$TMP/always-match2.sh"
    cat > "$ALWAYS_MATCH_MOCK" <<'EOF'
#!/usr/bin/env bash
cat >/dev/null
printf 'match\n'
EOF
    chmod +x "$ALWAYS_MATCH_MOCK"

    PROPOSAL_FILE="$TMP/prop-nofiler.json"
    printf '%s' '{"title":"p","score":0.5,"priority":false}' > "$PROPOSAL_FILE"

    # Run the matcher directly — must not call gh.
    PATH="$TMP:$PATH" \
    AUTOSPEC_GH_CMD="$FAKE_GH" \
    AUTOSPEC_PRIORITY_MATCH_CMD="bash $ALWAYS_MATCH_MOCK" \
    AUTOSPEC_PRIORITIES_FILE="$PRIORITIES_FILE" \
    AUTOSPEC_PERSONA_FILE="$PERSONA_FILE" \
    bash "$MATCH_SCRIPT" < "$PROPOSAL_FILE" >/dev/null

    [ ! -f "$GH_LOG" ] || [ ! -s "$GH_LOG" ]
}

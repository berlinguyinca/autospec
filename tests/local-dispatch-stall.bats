#!/usr/bin/env bats
# tests/local-dispatch-stall.bats — TDD for the local-dispatch no-progress abort
# (issue #3348).
#
# These are REAL bats tests driving a STUB EXECUTOR through the same PATH-stub
# mechanism tests/local-dispatch.bats uses — no mocks of the dispatcher itself.
# The stub is a fake `codex` on PATH whose behaviour is chosen by STUB_MODE:
#
#   silent      writes nothing, then waits to be killed
#   progressing writes one line per second, proving slow != stalled
#
# Both stubs idle with `sleep 1 & wait $!`, NOT a foreground `sleep 300`: bash
# runs a trapped signal only once the foreground command returns, so a foreground
# sleep would make the stub deaf to the guard's SIGTERM until it woke up.
#
# The three properties that matter:
#   1. a silent dispatch is killed with exit 5 (not 4, which means wall-clock),
#   2. a slow-but-progressing dispatch SURVIVES (the negative path — a guard
#      that aborts on latency would punish exactly the hardware this feature
#      exists to make usable),
#   3. the abort lands in the routing ledger as `local_overthink_abort` scoped to
#      AUTOSPEC_RUN_ID, and route-decide.sh escalates the cell to cloud on one
#      abort and demotes the profile run-wide on a second.

setup() {
    REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    SCRIPT="$REPO/scripts/local-dispatch.sh"
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/local-dispatch-stall-XXXXXX")"
    STUBS="$TMP/stubs"
    mkdir -p "$STUBS" "$TMP/home/.autospec"
    cat > "$TMP/home/.autospec/model-capability.json" <<'EOF'
{"local_models":[{"model":"qwen3:32b","dispatch_recommended":true}]}
EOF
    cat > "$TMP/prompt.txt" <<'EOF'
Do the thing.
EOF
    LEDGER="$TMP/routing-ledger.jsonl"
    RUN_ID="test-run-1"
    export STUB_DEATH_LOG="$TMP/death.log"
}

teardown() { rm -rf "$TMP"; }

# ── stub executor ─────────────────────────────────────────────────────────────
# Shared skeleton: advertises --oss (precondition 1) and records its death so a
# test can tell "the guard killed it" from "it exited by itself".
_stub_header() {
    cat <<'EOF'
#!/usr/bin/env bash
if printf '%s\n' "$@" | grep -q -- '--help'; then
    echo "  --oss                 run against a local provider"
    exit 0
fi
trap 'printf "terminated\n" >> "$STUB_DEATH_LOG"; exit 143' TERM
EOF
}

write_silent_stub() {
    _stub_header > "$STUBS/codex"
    cat >> "$STUBS/codex" <<'EOF'
# Idle in 1s slices so the TERM trap fires immediately. A bounded loop, not an
# unbounded wait: a stub that outlives its test is a leaked process.
_end=$(( $(date +%s) + 120 ))
while [ "$(date +%s)" -lt "$_end" ]; do sleep 1 & wait $!; done
exit 0
EOF
    chmod +x "$STUBS/codex"
}

write_progressing_stub() {
    _stub_header > "$STUBS/codex"
    cat >> "$STUBS/codex" <<'EOF'
_i=0
while [ "$_i" -lt 6 ]; do
    printf 'progress %s\n' "$_i"
    _i=$((_i+1))
    sleep 1 & wait $!
done
exit 0
EOF
    chmod +x "$STUBS/codex"
}

# ── dispatcher helpers ────────────────────────────────────────────────────────
# Runs the dispatcher against the stub. `run env ...` rather than `run VAR=x ...`
# because bats passes "$@" straight to exec, where a VAR=x prefix is argv[0], not
# an assignment. Bats then populates the `status` / `output` globals.
run_dispatch() {
    local stall="$1" ceiling="${2:-60}"
    shift 2
    run env PATH="$STUBS:$PATH" HOME="$TMP/home" \
        AUTOSPEC_ROUTING_LEDGER="$LEDGER" AUTOSPEC_RUN_ID="$RUN_ID" \
        bash "$SCRIPT" --skip-capability-check --stall-secs "$stall" \
        --timeout-secs "$ceiling" --model qwen3:32b --prompt-file "$TMP/prompt.txt" \
        --cwd "$TMP" --labels "ctx-64k,reasoning-medium" --profile qwen-local "$@"
}

# ── routing helpers ───────────────────────────────────────────────────────────
# A profile catalog whose names match what select-model-profile.sh resolves as
# the baseline (haiku for medium, sonnet for deep), plus one local profile
# (`cost_minute`, no token prices) that fits EVERY cell and is far cheaper than
# either cloud profile. So the local profile wins every cell by default, and any
# inversion below is provably the abort gate rather than a fitting difference.
setup_routing() {
    SELECTOR="$REPO/skills/autospec-run/scripts/select-model-profile.sh"
    DECIDE="$REPO/scripts/route-decide.sh"
    PROF="$TMP/profiles.yml"
    cat > "$PROF" <<'EOF'
claude-haiku-cloud:
  model: claude-haiku-4-5
  ctx: 120k
  reasoning: deep
  cost_in: 1.0
  cost_out: 5.0
claude-sonnet-cloud:
  model: claude-sonnet-4-6
  ctx: 120k
  reasoning: deep
  cost_in: 3.0
  cost_out: 15.0
qwen-local:
  model: qwen3:32b
  ctx: 120k
  reasoning: deep
  cost_minute: 0.02
EOF
    # Healthy evidence for all three cells for all three profiles, so the only
    # reason a local profile would stop winning is a no-progress abort.
    STATS="$TMP/stats.json"
    jq -n '[
      ["qwen-local","claude-haiku-cloud","claude-sonnet-cloud"] as $p
      | [["32k","shallow"],["64k","medium"],["120k","deep"]] as $c
      | $c[] as $cell | $p[] as $name
      | {dispatch_kind:"implementer", profile:$name, cell_ctx:$cell[0],
         cell_reasoning:$cell[1], dispatches:40, first_pass_rate:0.95,
         failure_rate:0.02, escalation_rate:0.02, mean_retries:0.2,
         cache_hit_ratio:0.5}
    ]' > "$STATS"
}

# route-decide.sh takes the cell as labels (`ctx:64k`, colon) and knows only
# --labels/--kind/--print-profile/--print-effort/--explain/--profiles-file/
# --stats-file.
decide() {
    env AUTOSPEC_ROUTING_LEDGER="$LEDGER" AUTOSPEC_RUN_ID="$RUN_ID" \
        AUTOSPEC_MODEL_PROFILES="$PROF" \
        bash "$DECIDE" --print-profile --kind implementer \
        --labels "ctx:$1,reasoning:$2" --profiles-file "$PROF" --stats-file "$STATS"
}

# append_abort <profile> <ctx> <reasoning> — an abort row as local-dispatch.sh
# writes one. Written through routing-ledger.sh itself, not hand-rolled JSON, so
# the ledger validates the row the same way production does.
append_abort() {
    local row
    row="$(jq -nc --arg p "$1" --arg c "$2" --arg r "$3" --arg run "$RUN_ID" \
        '{dispatch_id:("d-"+$p+$c+$r+(($run|tostring))),ts:"2026-08-09T00:00:00Z",
          dispatch_kind:"implementer",profile:$p,model:"qwen3:32b",harness:"codex-oss",
          issue:1,cell_ctx:$c,cell_reasoning:$r,input_tokens:0,output_tokens:0,
          cached_tokens:0,wall_clock_ms:120000,retries:0,escalated:true,
          outcome:"local_overthink_abort",reason:"no output for the stall window",
          run_id:$run}')"
    bash "$REPO/scripts/routing-ledger.sh" --ledger "$LEDGER" --append "$row" >/dev/null
}

# ══ 1. the dispatcher aborts a silent dispatch with exit 5 ══

@test "local-dispatch.sh kills a silent dispatch with exit 5, not 4" {
    write_silent_stub
    run_dispatch 2 60
    [ "$status" -eq 5 ]
    # The abort must be distinguishable in the log, not just in the number.
    [[ "$output" == *"local_overthink_abort"* ]]
    # Aborted inside the window plus one poll; a 60s ceiling means we never got near it.
    [ "$SECONDS" -lt 25 ]
}

@test "the abort terminates the executor instead of orphaning it" {
    write_silent_stub
    run_dispatch 2 60
    [ "$status" -eq 5 ]
    # Exit 5 without the executor seeing the signal would mean the guard gave up
    # and left a model holding the one GPU.
    [ -f "$STUB_DEATH_LOG" ]
    grep -q terminated "$STUB_DEATH_LOG"
}

@test "a slow but progressing dispatch is NOT aborted" {
    # The negative path, and the one that matters most: a 32B model on a laptop is
    # slow by definition. Only the ABSENCE of output may abort.
    write_progressing_stub
    run_dispatch 3 60
    [ "$status" -eq 0 ]
    [[ "$output" == *"progress 5"* ]]
    # The stub runs ~6s, longer than the 3s window: a latency-based guard fails here.
    [ "$SECONDS" -ge 5 ]
    [ ! -s "$LEDGER" ]
}

@test "wall-clock exhaustion still exits 4, not 5" {
    # The two guards must not be conflated: 4 = ran out of time, 5 = stopped
    # making progress, and only 5 demotes a model.
    write_silent_stub
    run_dispatch 60 2
    [ "$status" -eq 4 ]
    [[ "$output" == *"exceeded 2s ceiling"* ]]
    [ ! -s "$LEDGER" ]
}

@test "AUTOSPEC_LOCAL_STALL_SECS drives the window and --stall-secs overrides it" {
    write_silent_stub
    RUN_ID="run-env" LEDGER="$TMP/env.jsonl"
    run env PATH="$STUBS:$PATH" HOME="$TMP/home" \
        AUTOSPEC_ROUTING_LEDGER="$LEDGER" AUTOSPEC_RUN_ID="$RUN_ID" \
        AUTOSPEC_LOCAL_STALL_SECS=1 \
        bash "$SCRIPT" --skip-capability-check --timeout-secs 60 \
        --model qwen3:32b --prompt-file "$TMP/prompt.txt" --cwd "$TMP"
    [ "$status" -eq 5 ]

    # An explicit --stall-secs 120 must win over the env default of 1s, otherwise
    # the env var cannot be tuned globally without breaking a one-off caller.
    write_silent_stub
    run env PATH="$STUBS:$PATH" HOME="$TMP/home" \
        AUTOSPEC_ROUTING_LEDGER="$LEDGER" AUTOSPEC_RUN_ID="$RUN_ID" \
        AUTOSPEC_LOCAL_STALL_SECS=1 \
        bash "$SCRIPT" --skip-capability-check --stall-secs 120 --timeout-secs 3 \
        --model qwen3:32b --prompt-file "$TMP/prompt.txt" --cwd "$TMP"
    [ "$status" -eq 4 ]
}

@test "--stall-secs rejects a non-integer and a zero window" {
    write_silent_stub
    for bad in abc 0 -5; do
        run env PATH="$STUBS:$PATH" HOME="$TMP/home" bash "$SCRIPT" \
            --skip-capability-check --stall-secs "$bad" --model qwen3:32b \
            --prompt-file "$TMP/prompt.txt" --cwd "$TMP"
        [ "$status" -eq 1 ]
        [[ "$output" == *"stall-secs"* ]]
    done
}

@test "--dry-run reports the stall window without dispatching" {
    write_silent_stub
    run env PATH="$STUBS:$PATH" HOME="$TMP/home" bash "$SCRIPT" \
        --skip-capability-check --dry-run --model qwen3:32b \
        --prompt-file "$TMP/prompt.txt" --stall-secs 7
    [ "$status" -eq 0 ]
    [[ "$output" == *"stall_secs=7"* ]]
    [ ! -e "$STUB_DEATH_LOG" ]
}

# ══ 2. the abort is recorded in the routing ledger ══

@test "a no-progress abort appends local_overthink_abort to the ledger" {
    write_silent_stub
    run_dispatch 2 60
    [ "$status" -eq 5 ]
    [ -f "$LEDGER" ]
    run jq -e 'select(.outcome == "local_overthink_abort")' "$LEDGER"
    [ "$status" -eq 0 ]
    [ "$(wc -l < "$LEDGER" | tr -d ' ')" -eq 1 ]
}

@test "the abort row carries the cell and run it happened in" {
    write_silent_stub
    run_dispatch 2 60
    [ "$status" -eq 5 ]
    # Without the cell, route-decide.sh cannot escalate THAT cell; without the
    # run id, it cannot tell this run's stall from one three weeks ago.
    run jq -e '.cell_ctx == "64k" and .cell_reasoning == "medium"
               and .run_id == "test-run-1" and .profile == "qwen-local"' "$LEDGER"
    [ "$status" -eq 0 ]
    # Validate the row through the ledger's own validator, not just jq.
    run bash "$REPO/scripts/routing-ledger.sh" --ledger "$LEDGER" --validate
    [ "$status" -eq 0 ]
}

@test "local_overthink_abort is an accepted outcome in the ledger vocabulary" {
    # routing-ledger.sh rejects unknown outcomes; the new outcome must be in the
    # allowed set or every abort row silently fails to land.
    run grep -E '^\s*ALLOWED_OUTCOMES=' "$REPO/scripts/routing-ledger.sh"
    [ "$status" -eq 0 ]
    [[ "$output" == *"local_overthink_abort"* ]]
    run bash "$REPO/scripts/routing-ledger.sh" --ledger "$TMP/vocab.jsonl" --append \
        "$(jq -nc '{dispatch_id:"v1",ts:"2026-08-09T00:00:00Z",dispatch_kind:"implementer",
                    profile:"qwen-local",model:"qwen3:32b",harness:"codex-oss",issue:1,
                    cell_ctx:"64k",cell_reasoning:"medium",input_tokens:0,output_tokens:0,
                    cached_tokens:0,wall_clock_ms:1000,retries:0,escalated:true,
                    outcome:"local_overthink_abort",reason:"stalled"}')"
    [ "$status" -eq 0 ]
}

@test "a stall with no ledger configured still exits 5 and invents no ledger" {
    write_silent_stub
    LEDGER=""
    run_dispatch 2 60
    [ "$status" -eq 5 ]
    [[ "$output" == *"local_overthink_abort"* ]]
    # Never create a ledger inside the caller's cwd as a side effect.
    [ ! -e "$TMP/cwd/.autospec/routing-ledger.jsonl" ]
    [ ! -e "$TMP/routing-ledger.jsonl" ]
}

@test "an abort with unknown cell ordinals is not recorded against a fake cell" {
    # A row filed at a guessed cell poisons the evidence for dispatches that never
    # happened there and would demote a model that did nothing wrong.
    write_silent_stub
    run env PATH="$STUBS:$PATH" HOME="$TMP/home" \
        AUTOSPEC_ROUTING_LEDGER="$LEDGER" AUTOSPEC_RUN_ID="$RUN_ID" \
        bash "$SCRIPT" --skip-capability-check --stall-secs 2 --timeout-secs 60 \
        --model qwen3:32b --prompt-file "$TMP/prompt.txt" --cwd "$TMP"
    [ "$status" -eq 5 ]
    [ ! -s "$LEDGER" ]
    [[ "$output" == *"not recorded"* ]]
}

# ══ 3. route-decide.sh escalates the cell, then demotes the profile ══

@test "a healthy cell routes to the cheap local profile" {
    setup_routing
    run decide 64k medium
    [ "$status" -eq 0 ]
    [ "$output" = "qwen-local" ]
}

@test "one abort escalates THAT cell to cloud for the rest of the run" {
    setup_routing
    append_abort qwen-local 64k medium
    run decide 64k medium
    [ "$status" -eq 0 ]
    [ "$output" = "claude-haiku-cloud" ]

    # The neighbouring cell is untouched: one stalled dispatch is not evidence
    # about a different cell.
    run decide 32k shallow
    [ "$output" = "qwen-local" ]

    # And the same cell recovers next run — this is within-run state, not a
    # permanent blacklist.
    run env AUTOSPEC_ROUTING_LEDGER="$LEDGER" AUTOSPEC_RUN_ID="other-run-2" \
        AUTOSPEC_MODEL_PROFILES="$PROF" \
        bash "$DECIDE" --print-profile --kind implementer --labels "ctx:64k,reasoning:medium" \
        --profiles-file "$PROF" --stats-file "$STATS"
    [ "$output" = "qwen-local" ]
}

@test "a second abort anywhere in the run demotes the profile run-wide" {
    setup_routing
    append_abort qwen-local 64k medium
    append_abort qwen-local 32k shallow
    run decide 64k medium
    [ "$output" = "claude-haiku-cloud" ]
    run decide 32k shallow
    [ "$output" = "claude-haiku-cloud" ]
    # Including a cell that never aborted at all: run-wide demotion means the
    # profile is offered nowhere, and the cheapest remaining cloud profile wins
    # whatever the cell's own baseline would have been.
    run decide 120k deep
    [ "$output" != "qwen-local" ]
    [ "$output" = "claude-haiku-cloud" ]
}

@test "an abort in a previous run does not escalate this run" {
    setup_routing
    RUN_ID="run-1"
    append_abort qwen-local 64k medium
    run env AUTOSPEC_ROUTING_LEDGER="$LEDGER" AUTOSPEC_RUN_ID="run-2" \
        AUTOSPEC_MODEL_PROFILES="$PROF" \
        bash "$DECIDE" --print-profile --kind implementer --labels "ctx:64k,reasoning:medium" \
        --profiles-file "$PROF" --stats-file "$STATS"
    [ "$output" = "qwen-local" ]
}

@test "an abort filed against a CLOUD profile does not evict the local tier" {
    # Gate 1 is about the local tier stalling. A row written against a cloud
    # profile says nothing about whether the user's own GPU loops on this cell, and
    # demoting local on that evidence would cost the operator a tier they never
    # broke.
    setup_routing
    append_abort claude-haiku-cloud 64k medium
    append_abort claude-haiku-cloud 64k medium
    run decide 64k medium
    [ "$output" = "qwen-local" ]
}

@test "route-decide.sh logs the no-progress veto as local_overthink_abort" {
    setup_routing
    append_abort qwen-local 64k medium
    run env AUTOSPEC_ROUTING_LEDGER="$LEDGER" AUTOSPEC_RUN_ID="$RUN_ID" \
        AUTOSPEC_MODEL_PROFILES="$PROF" \
        bash "$DECIDE" --explain --kind implementer --labels "ctx:64k,reasoning:medium" \
        --profiles-file "$PROF" --stats-file "$STATS"
    [ "$status" -eq 0 ]
    [[ "$output" == *"local_overthink_abort"* ]]
}

@test "the abort gate cannot resurrect a vetoed baseline through the cost gate" {
    # The strictly-cheaper gate compares candidates against the baseline. If the
    # baseline itself was just vetoed, the gate must not use its cheap price to
    # put it back.
    setup_routing
    append_abort qwen-local 64k medium
    append_abort qwen-local 64k medium
    run decide 64k medium
    [ "$output" != "qwen-local" ]
}

# ══ 4. documentation surface ══

@test "AUTOSPEC_LOCAL_STALL_SECS is documented with exit 5" {
    grep -q '`AUTOSPEC_LOCAL_STALL_SECS`' "$REPO/docs/CONFIG_REFERENCE.md"
    grep -q 'local_overthink_abort' "$REPO/docs/CONFIG_REFERENCE.md"
    grep -Eq 'exit 5|exits 5|`5`' "$REPO/docs/CONFIG_REFERENCE.md"
}

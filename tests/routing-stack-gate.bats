#!/usr/bin/env bats
# tests/routing-stack-gate.bats — TDD for the R11 per-stack local-eligibility
# gate (tracker #3344) and the routing-ledger `stack` field it reads.
#
# The gate has two independent refusals, in priority order:
#   * deliverable — non-code deliverables (document/LaTeX generation, non-English
#     output, breadth-of-knowledge research) are NEVER local-eligible, no matter
#     what the ledger says. This half is always active.
#   * stack evidence — active only when a stack is detected (--stack flag or
#     .autospec/state/stack-profile.json in the working directory). A local
#     profile is eligible only if the routing ledger holds at least one
#     successful outcome from a local profile on THIS exact stack. Default deny:
#     unrecognized stacks, an empty id, and an empty ledger all refuse.
#
# The load-bearing parity property still holds: with no stack detected at all
# (no --stack, no stack-profile.json) and an empty or thin ledger,
# route-decide.sh prints exactly what select-model-profile.sh prints —
# "no data = no change".

DECIDE="${BATS_TEST_DIRNAME}/../scripts/route-decide.sh"
LEDGER_SH="${BATS_TEST_DIRNAME}/../scripts/routing-ledger.sh"
DETECT="${BATS_TEST_DIRNAME}/../scripts/autospec-detect-stack-profile.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/routing-stack-gate-XXXXXX")"
    ORIG_PWD="$(pwd)"
    # Every test runs inside a clean directory: the repository's own
    # .autospec/ state can never leak in and activate (or pollute) the gate.
    cd "$TMP"

    EMPTY="$TMP/empty.json"
    printf '[]' > "$EMPTY"
    LEDGER="$TMP/ledger.jsonl"

    PROF="$TMP/profiles.yml"
    cat > "$PROF" <<'EOF'
claude-haiku-cloud:
  model: claude-haiku-4-5
  ctx: 64k
  reasoning: medium
  cost_in: 1.0
  cost_out: 5.0
claude-sonnet-cloud:
  model: claude-sonnet-4-6
  ctx: 120k
  reasoning: deep
  cost_in: 3.0
  cost_out: 15.0
qwen3-32b-laptop:
  model: qwen3:32b
  ctx: 120k
  reasoning: deep
  cost_minute: 0.02
EOF
}

teardown() {
    cd "$ORIG_PWD"
    rm -rf "$TMP"
}

# stats_row <profile> <ctx> <reasoning> <n> <first_pass> <fail> <esc> <retries> <cache>
stats_row() {
    printf '{"dispatch_kind":"implementer","profile":"%s","cell_ctx":"%s","cell_reasoning":"%s","dispatches":%s,"first_pass_rate":%s,"failure_rate":%s,"escalation_rate":%s,"mean_retries":%s,"cache_hit_ratio":%s}' \
        "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9"
}

# rec <id> <kind> <profile> <reasoning> <in> <cached> <ms> <retries> <esc> <outcome> [stack]
# A minimal valid ledger record. The 11th argument (stack) is optional, so the
# no-stack shape still exercises the legacy append path.
rec() {
    local extra=""
    if [ -n "${11:-}" ]; then extra=",\"stack\":\"${11}\""; fi
    printf '{"dispatch_id":"%s","ts":"2026-08-05T10:00:00Z","dispatch_kind":"%s","profile":"%s","model":"m","harness":"claude","issue":1,"cell_ctx":"64k","cell_reasoning":"%s","input_tokens":%s,"output_tokens":10,"cached_tokens":%s,"wall_clock_ms":%s,"retries":%s,"escalated":%s,"outcome":"%s","reason":""%s}' \
        "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "${10}" "$extra"
}

# strong_local_stats — the fixture that makes qwen win the 120k/deep cell when
# the gate lets it in (mirrors tests/routing-decision.bats).
strong_local_stats() {
    jq -n --argjson a "$(stats_row qwen3-32b-laptop 120k deep 50 0.90 0.05 0.05 0.2 0.0)" \
          --argjson b "$(stats_row claude-sonnet-cloud 120k deep 50 0.92 0.04 0.02 0.15 0.8)" \
        '[$a,$b]' > "$TMP/s.json"
}

# seed_ledger <stack> <profile> <outcome> <dispatch_id>
# Appends one raw ledger row and fails the test if the row did not land.
seed_ledger() {
    bash "$LEDGER_SH" --ledger "$LEDGER" --append \
        "$(rec "$4" implementer "$2" shallow 100 0 10 0 false "$3" "$1")" || return 1
    [ "$(bash "$LEDGER_SH" --ledger "$LEDGER" --show --json | jq 'length')" -ge 1 ]
}

# decide [extra route-decide flags]
# Route the shared 120k/deep implementer cell. AUTOSPEC_ROUTING_LEDGER is always
# pointed at the temp ledger so a repo-root ledger can never answer the
# evidence query.
decide() {
    env AUTOSPEC_MODEL_PROFILES="$PROF" AUTOSPEC_ROUTING_LEDGER="$LEDGER" \
        bash "$DECIDE" --profiles-file "$PROF" --labels "auto-implement,reasoning:deep,ctx:120k" \
        --stats-file "$TMP/s.json" "$@"
}

# ── routing-ledger.sh: the stack field ────────────────────────────────────────

@test "append normalizes a missing stack to unknown" {
    run bash "$LEDGER_SH" --ledger "$LEDGER" --append \
        "$(rec d1 implementer qwen3-32b-laptop shallow 100 0 10 0 false merged_clean)"
    [ "$status" -eq 0 ]
    run bash "$LEDGER_SH" --ledger "$LEDGER" --show --json
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.[0].stack')" = "unknown" ]
}

@test "append records an explicit stack" {
    run bash "$LEDGER_SH" --ledger "$LEDGER" --append \
        "$(rec d1 implementer qwen3-32b-laptop shallow 100 0 10 0 false merged_clean typescript)"
    [ "$status" -eq 0 ]
    run bash "$LEDGER_SH" --ledger "$LEDGER" --show --json
    [ "$(printf '%s' "$output" | jq -r '.[0].stack')" = "typescript" ]
}

@test "append rejects a non-string stack" {
    bad="$(rec d1 implementer qwen3-32b-laptop shallow 100 0 10 0 false merged_clean | jq -c '. + {stack: 42}')"
    run bash "$LEDGER_SH" --ledger "$LEDGER" --append "$bad"
    [ "$status" -ne 0 ]
    [[ "$output" == *"stack"* ]]
}

@test "append rejects an empty-string stack" {
    bad="$(rec d1 implementer qwen3-32b-laptop shallow 100 0 10 0 false merged_clean | jq -c '. + {stack: ""}')"
    run bash "$LEDGER_SH" --ledger "$LEDGER" --append "$bad"
    [ "$status" -ne 0 ]
    [[ "$output" == *"stack"* ]]
}

@test "update-outcome normalizes a missing stack to unknown" {
    bash "$LEDGER_SH" --ledger "$LEDGER" --append \
        "$(rec d1 implementer qwen3-32b-laptop shallow 100 0 10 0 false pending)"
    run bash "$LEDGER_SH" --ledger "$LEDGER" --update-outcome d1 merged_clean
    [ "$status" -eq 0 ]
    run bash "$LEDGER_SH" --ledger "$LEDGER" --show --json
    [ "$(printf '%s' "$output" | jq -r '.[0].stack')" = "unknown" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].outcome')" = "merged_clean" ]
}

@test "validate accepts a legacy row without a stack key" {
    bash "$LEDGER_SH" --ledger "$LEDGER" --append \
        "$(rec d1 implementer qwen3-32b-laptop shallow 100 0 10 0 false merged_clean typescript)"
    # A pre-stack row (no key at all) must still validate: the field is
    # optional at rest, only normalized on the way in.
    rec d2 implementer qwen3-32b-laptop shallow 100 0 10 0 false merged_clean >> "$LEDGER"
    run bash "$LEDGER_SH" --validate "$LEDGER"
    [ "$status" -eq 0 ]
}

# ── gate dormant: parity must hold ────────────────────────────────────────────

@test "no stack detected: an earned local profile still wins (parity)" {
    strong_local_stats
    run decide
    [ "$status" -eq 0 ]
    [ "$output" = "qwen3:32b" ]
}

@test "no stack detected: an empty ledger still routes to the baseline (parity)" {
    printf '[]' > "$TMP/s.json"
    run decide
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

# ── stack evidence: default deny ─────────────────────────────────────────────

@test "recognized stack with no ledger evidence denies local" {
    strong_local_stats
    run decide --stack typescript
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "evidence on a different stack does not count" {
    strong_local_stats
    seed_ledger python qwen3-32b-laptop merged_clean d1
    run decide --stack typescript
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "a failed local outcome is not success evidence" {
    strong_local_stats
    seed_ledger typescript qwen3-32b-laptop qa_failed d1
    run decide --stack typescript
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "a cloud success on the stack is not local evidence" {
    strong_local_stats
    seed_ledger typescript claude-sonnet-cloud merged_clean d1
    run decide --stack typescript
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "an explicit unknown stack denies local even with an unknown-stack row" {
    strong_local_stats
    seed_ledger unknown qwen3-32b-laptop merged_clean d1
    run decide --stack unknown
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "an empty stack value denies local (fail closed)" {
    strong_local_stats
    run decide --stack ""
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

# ── stack evidence: allow only on the matching stack ─────────────────────────

@test "a successful local outcome on the detected stack admits local" {
    strong_local_stats
    seed_ledger typescript qwen3-32b-laptop merged_clean d1
    run decide --stack typescript
    [ "$status" -eq 0 ]
    [ "$output" = "qwen3:32b" ]
}

@test "retried_ok also counts as success evidence" {
    strong_local_stats
    seed_ledger typescript qwen3-32b-laptop retried_ok d1
    run decide --stack typescript
    [ "$status" -eq 0 ]
    [ "$output" = "qwen3:32b" ]
}

# ── stack resolved from .autospec/state/stack-profile.json ───────────────────

@test "stack-profile.json with an unknown id activates the gate and denies" {
    strong_local_stats
    mkdir -p .autospec/state
    printf '{"schema":1,"profiles":[],"languages":[],"frameworks":[],"primary_profile":{"id":"unknown","confidence":0.1}}\n' \
        > .autospec/state/stack-profile.json
    run decide
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "stack-profile.json with a detected id admits local when the ledger agrees" {
    strong_local_stats
    mkdir -p .autospec/state
    printf '{"schema":1,"profiles":[],"languages":[],"frameworks":[],"primary_profile":{"id":"typescript","confidence":0.8}}\n' \
        > .autospec/state/stack-profile.json
    seed_ledger typescript qwen3-32b-laptop lgtm_first_pass d1
    run decide
    [ "$status" -eq 0 ]
    [ "$output" = "qwen3:32b" ]
}

@test "a malformed stack-profile.json activates the gate and denies" {
    strong_local_stats
    mkdir -p .autospec/state
    printf 'not json' > .autospec/state/stack-profile.json
    run decide
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

# ── deliverable refusal: always active ───────────────────────────────────────

@test "a non-code deliverable is never local-eligible, even with stack evidence" {
    strong_local_stats
    seed_ledger typescript qwen3-32b-laptop merged_clean d1
    run decide --stack typescript --deliverable document
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "every non-code deliverable kind denies local" {
    strong_local_stats
    seed_ledger typescript qwen3-32b-laptop merged_clean d1
    for d in document latex translation research; do
        run decide --stack typescript --deliverable "$d"
        [ "$status" -eq 0 ]
        [ "$output" = "claude-sonnet-4-6" ]
    done
}

@test "an unknown deliverable value fails closed" {
    strong_local_stats
    seed_ledger typescript qwen3-32b-laptop merged_clean d1
    run decide --stack typescript --deliverable spreadsheet
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "a non-code deliverable denies local even when the stack gate is dormant" {
    strong_local_stats
    # No --stack, no stack-profile.json: the stack-evidence half is dormant,
    # but the deliverable half is always active.
    run decide --deliverable research
    [ "$status" -eq 0 ]
    [ "$output" = "claude-sonnet-4-6" ]
}

@test "a code deliverable with stack evidence still admits local" {
    strong_local_stats
    seed_ledger typescript qwen3-32b-laptop merged_clean d1
    run decide --stack typescript --deliverable code
    [ "$status" -eq 0 ]
    [ "$output" = "qwen3:32b" ]
}

# ── autospec-detect-stack-profile.sh --print-stack ───────────────────────────

@test "--print-stack prints the detected primary profile id" {
    mkdir -p repo
    printf '[project]\nname = "x"\n' > repo/pyproject.toml
    printf 'a = 1\nb = 2\n' > repo/x.py
    run bash "$DETECT" --repo-root "$TMP/repo" --print-stack
    [ "$status" -eq 0 ]
    [ "$output" = "python" ]
    [ -f "$TMP/repo/.autospec/state/stack-profile.json" ]
}

@test "--print-stack prints unknown for a repo without markers" {
    mkdir -p repo
    printf 'readme\n' > repo/README.md
    run bash "$DETECT" --repo-root "$TMP/repo" --print-stack
    [ "$status" -eq 0 ]
    [ "$output" = "unknown" ]
}

@test "without --print-stack the script still prints nothing" {
    mkdir -p repo
    printf '[project]\nname = "x"\n' > repo/pyproject.toml
    printf 'a = 1\nb = 2\n' > repo/x.py
    run bash "$DETECT" --repo-root "$TMP/repo"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
    [ -f "$TMP/repo/.autospec/state/stack-profile.json" ]
}

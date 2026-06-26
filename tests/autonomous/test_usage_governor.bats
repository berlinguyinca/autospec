#!/usr/bin/env bats
# tests/autonomous/test_usage_governor.bats — unit tests for
# scripts/autonomous-usage-governor.sh (F6b soft-park at 90%).
#
# Covers:
#  - observable fraction at threshold parks
#  - observable fraction below threshold continues
#  - unobservable harness: spend-ledger tally at 90% of lifetime parks
#  - unobservable harness: spend-ledger tally below 90% continues
#  - park arms autospec-usage-limit.sh with arm subcommand
#  - park calls notify.sh
#  - disabled lifetime cap (LIFETIME_TOKENS=0) → continue
#  - custom AUTOSPEC_USAGE_SOFT_PCT respected
#  - macOS bash 3.2 compat: real temp files before [ -f ] in helpers

setup() {
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    SCRIPT="$REPO_ROOT/scripts/autonomous-usage-governor.sh"

    TEST_DIR="$(mktemp -d)"
    export TEST_DIR

    NOTIFY_LOG="$TEST_DIR/notify.log"
    USAGE_LIMIT_LOG="$TEST_DIR/usage-limit.log"
    export NOTIFY_LOG USAGE_LIMIT_LOG

    # Mock notify.sh — record calls to a real temp file.
    _write_notify_mock

    # Mock autospec-usage-limit.sh — record args to a real temp file.
    _write_usage_limit_mock

    # Default mock for usage-observe.sh: unobservable (covers the fallback path).
    _write_observe_mock_unobservable

    # Default mock for autonomous-spend-ledger.sh: 0 tokens (under any threshold).
    _write_ledger_mock 0

    export PATH="$TEST_DIR:$PATH"

    # Fake repo dir (not a real git repo; governor does not require one).
    REPO_DIR="$TEST_DIR/fake-repo"
    mkdir -p "$REPO_DIR"
    export REPO_DIR
}

teardown() {
    rm -rf "$TEST_DIR"
}

# ── Mock helpers ─────────────────────────────────────────────────────────────
# Each helper writes to a real file in TEST_DIR (bash 3.2 compat: never [ -f <(...)]).

_write_notify_mock() {
    local path="$TEST_DIR/notify.sh"
    local log="$NOTIFY_LOG"
    # Unquoted heredoc: ${log} expands here; \$1 / \${2:-} are escaped to stay
    # literal in the generated script.  \\t / \\n become \t / \n in the file.
    cat > "$path" <<SH
#!/usr/bin/env bash
printf '%s\\t%s\\n' "\$1" "\${2:-}" >> "${log}"
SH
    # Verify write before chmod (feedback_bash32_process_sub_test_file).
    [ -f "$path" ]
    chmod +x "$path"
}

_write_usage_limit_mock() {
    local path="$TEST_DIR/autospec-usage-limit.sh"
    local log="$USAGE_LIMIT_LOG"
    cat > "$path" <<SH
#!/usr/bin/env bash
printf '%s\\n' "\$*" >> "${log}"
SH
    [ -f "$path" ]
    chmod +x "$path"
}

_write_observe_mock_unobservable() {
    local path="$TEST_DIR/usage-observe.sh"
    cat > "$path" <<'SH'
#!/usr/bin/env bash
printf '{"harness":"%s","observable":false,"percent":null,"source":"mock"}\n' "${1:-claude}"
SH
    [ -f "$path" ]
    chmod +x "$path"
}

_write_observe_mock_observable() {
    local pct="$1"
    local path="$TEST_DIR/usage-observe.sh"
    # Unquoted heredoc: ${pct} expands here (wanted); \${1:-claude} is escaped
    # so the generated script receives it as a literal shell parameter expansion.
    cat > "$path" <<SH
#!/usr/bin/env bash
printf '{"harness":"%s","observable":true,"percent":${pct},"source":"mock"}\\n' "\${1:-claude}"
SH
    [ -f "$path" ]
    chmod +x "$path"
}

_write_ledger_mock() {
    local tokens="$1"
    local path="$TEST_DIR/autonomous-spend-ledger.sh"
    cat > "$path" <<SH
#!/usr/bin/env bash
printf '{"schema":1,"tokens":${tokens},"issues":0,"parked":false}\\n'
SH
    [ -f "$path" ]
    chmod +x "$path"
}

run_governor() {
    run bash "$SCRIPT" "$@" --repo-dir "$REPO_DIR"
}

# ── Observable fraction: park at/above threshold ──────────────────────────────
# bats `run` captures stderr (via 2>&1) so $output may contain info() lines
# before the "park ..." decision line.  Use grep per-line, not `== park*`.

@test "observable fraction exactly at 90% soft threshold parks" {
    _write_observe_mock_observable 90
    run_governor claude
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
}

@test "observable fraction above 90% (95%) parks" {
    _write_observe_mock_observable 95
    run_governor claude
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
}

@test "observable fraction at 100% parks" {
    _write_observe_mock_observable 100
    run_governor claude
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
}

# ── Observable fraction: continue below threshold ─────────────────────────────

@test "observable fraction below 90% (89%) continues" {
    _write_observe_mock_observable 89
    run_governor claude
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

@test "observable fraction at 0% continues" {
    _write_observe_mock_observable 0
    run_governor claude
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

@test "observable fraction at 89.9% continues (decimal below threshold)" {
    _write_observe_mock_observable 89.9
    run_governor codex
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

# ── Custom AUTOSPEC_USAGE_SOFT_PCT ────────────────────────────────────────────

@test "custom AUTOSPEC_USAGE_SOFT_PCT=50 parks when observable fraction is 50%" {
    _write_observe_mock_observable 50
    AUTOSPEC_USAGE_SOFT_PCT=50 run_governor claude
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
}

@test "custom AUTOSPEC_USAGE_SOFT_PCT=50 continues when observable fraction is 49%" {
    _write_observe_mock_observable 49
    AUTOSPEC_USAGE_SOFT_PCT=50 run_governor claude
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

# ── Spend-ledger tally fallback (unobservable harness) ────────────────────────

@test "unobservable harness: tokens at 90% of lifetime parks (tally fallback)" {
    # 9000000 = 90% of 10000000
    _write_ledger_mock 9000000
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=10000000 run_governor claude
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
}

@test "unobservable harness: tokens above 90% of lifetime parks" {
    # 9500000 > 90% of 10000000
    _write_ledger_mock 9500000
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=10000000 run_governor claude
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
}

@test "unobservable harness: tokens below 90% of lifetime continues" {
    # 8999999 < 90% of 10000000
    _write_ledger_mock 8999999
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=10000000 run_governor claude
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

@test "unobservable harness: 0 tokens continues" {
    _write_ledger_mock 0
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=10000000 run_governor claude
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

@test "unobservable harness: tally fallback with custom AUTOSPEC_USAGE_SOFT_PCT=80" {
    # 8000 = 80% of 10000
    _write_ledger_mock 8000
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=10000 \
    AUTOSPEC_USAGE_SOFT_PCT=80 \
    run_governor opencode
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
}

# ── Disabled lifetime token cap (LIFETIME_TOKENS=0 → no soft park) ────────────

@test "LIFETIME_TOKENS=0 disables the tally-fallback soft cap (continue)" {
    # Even at very high token count, a 0 lifetime means disabled.
    _write_ledger_mock 99999999
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=0 run_governor claude
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

# ── Park side-effects: notify.sh ──────────────────────────────────────────────

@test "park calls notify.sh (observable path)" {
    _write_observe_mock_observable 90
    run_governor claude
    [ "$status" -eq 0 ]
    # Write notify log path to a real temp file before [ -f ] (bash 3.2 compat).
    tmp_check="$(mktemp)"
    [ -f "$NOTIFY_LOG" ] && printf 'yes' > "$tmp_check" || printf 'no' > "$tmp_check"
    result="$(cat "$tmp_check")"
    rm -f "$tmp_check"
    [ "$result" = "yes" ]
    grep -q "autospec-autonomous soft-park" "$NOTIFY_LOG"
}

@test "park calls notify.sh (tally fallback path)" {
    _write_ledger_mock 9000000
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=10000000 run_governor claude
    [ "$status" -eq 0 ]
    tmp_check="$(mktemp)"
    [ -f "$NOTIFY_LOG" ] && printf 'yes' > "$tmp_check" || printf 'no' > "$tmp_check"
    result="$(cat "$tmp_check")"
    rm -f "$tmp_check"
    [ "$result" = "yes" ]
    grep -q "autospec-autonomous soft-park" "$NOTIFY_LOG"
}

@test "continue does NOT call notify.sh" {
    _write_observe_mock_observable 50
    run_governor claude
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
    # notify log must NOT exist.
    tmp_check="$(mktemp)"
    [ -f "$NOTIFY_LOG" ] && printf 'yes' > "$tmp_check" || printf 'no' > "$tmp_check"
    result="$(cat "$tmp_check")"
    rm -f "$tmp_check"
    [ "$result" = "no" ]
}

# ── Park side-effects: autospec-usage-limit.sh arm ───────────────────────────

@test "park arms autospec-usage-limit.sh when --resume-command is supplied" {
    _write_observe_mock_observable 90
    run bash "$SCRIPT" claude \
        --repo-dir "$REPO_DIR" \
        --resume-command "autospec-autonomous resume" \
        --wait-seconds 3600
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
    # Write check to real temp file (bash 3.2 compat).
    tmp_check="$(mktemp)"
    [ -f "$USAGE_LIMIT_LOG" ] && printf 'yes' > "$tmp_check" || printf 'no' > "$tmp_check"
    result="$(cat "$tmp_check")"
    rm -f "$tmp_check"
    [ "$result" = "yes" ]
    grep -q "arm" "$USAGE_LIMIT_LOG"
    grep -q "autospec-autonomous resume" "$USAGE_LIMIT_LOG"
}

@test "park WITHOUT --resume-command does NOT call autospec-usage-limit.sh" {
    _write_observe_mock_observable 90
    run_governor claude
    [ "$status" -eq 0 ]
    [[ "$output" == park* ]]
    tmp_check="$(mktemp)"
    [ -f "$USAGE_LIMIT_LOG" ] && printf 'yes' > "$tmp_check" || printf 'no' > "$tmp_check"
    result="$(cat "$tmp_check")"
    rm -f "$tmp_check"
    [ "$result" = "no" ]
}

@test "park passes --run-id to autospec-usage-limit.sh when supplied" {
    _write_observe_mock_observable 90
    run bash "$SCRIPT" claude \
        --repo-dir "$REPO_DIR" \
        --resume-command "autospec-autonomous resume" \
        --run-id "test-run-42" \
        --wait-seconds 3600
    [ "$status" -eq 0 ]
    grep -q "test-run-42" "$USAGE_LIMIT_LOG"
}

# ── park output format ────────────────────────────────────────────────────────

@test "park output starts with 'park ' followed by an ISO8601 timestamp" {
    _write_observe_mock_observable 90
    run_governor claude --wait-seconds 3600
    [ "$status" -eq 0 ]
    # Output: "park 2026-06-25T..."
    printf '%s' "$output" | grep -Eq '^park [0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$'
}

@test "park output with explicit --resume-at preserves the supplied timestamp" {
    _write_observe_mock_observable 90
    run bash "$SCRIPT" claude \
        --repo-dir "$REPO_DIR" \
        --resume-at "2099-01-01T00:00:00Z"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -qF 'park 2099-01-01T00:00:00Z'
}

# ── Harness variants ──────────────────────────────────────────────────────────

@test "codex harness observable park works" {
    _write_observe_mock_observable 90
    run bash "$SCRIPT" codex --repo-dir "$REPO_DIR"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
}

@test "opencode harness observable park works" {
    _write_observe_mock_observable 90
    run bash "$SCRIPT" opencode --repo-dir "$REPO_DIR"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
}

@test "unknown harness exits non-zero" {
    run bash "$SCRIPT" bogus --repo-dir "$REPO_DIR"
    [ "$status" -ne 0 ]
}

@test "missing harness exits non-zero" {
    run bash "$SCRIPT"
    [ "$status" -ne 0 ]
}

# ── Resilience: observe probe failure falls through to ledger ─────────────────

@test "if usage-observe.sh exits non-zero, falls back to ledger tally" {
    # Make observe exit 1.
    cat > "$TEST_DIR/usage-observe.sh" <<'SH'
#!/usr/bin/env bash
exit 1
SH
    chmod +x "$TEST_DIR/usage-observe.sh"

    _write_ledger_mock 9000000
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=10000000 run_governor claude
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
}

@test "if both observe and ledger fail, emits continue (fail-open)" {
    # Observe fails.
    cat > "$TEST_DIR/usage-observe.sh" <<'SH'
#!/usr/bin/env bash
exit 1
SH
    chmod +x "$TEST_DIR/usage-observe.sh"
    # Ledger fails.
    cat > "$TEST_DIR/autonomous-spend-ledger.sh" <<'SH'
#!/usr/bin/env bash
exit 1
SH
    chmod +x "$TEST_DIR/autonomous-spend-ledger.sh"

    run_governor claude
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

# ── Regression: malformed-JSON fail-open (#1409 hardening) ────────────────────
# A safety governor must never abort with no verdict. Before the jq guards, a
# malformed ledger/observe payload under set -e aborted the script (rc 5) with
# empty stdout, breaking the advertised continue|park contract.

@test "malformed observe JSON falls through to ledger tally (fail-open, rc 0)" {
    cat > "$TEST_DIR/usage-observe.sh" <<'SH'
#!/usr/bin/env bash
echo "this is not json {{{"
SH
    chmod +x "$TEST_DIR/usage-observe.sh"

    _write_ledger_mock 9000000
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=10000000 run_governor claude
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
}

@test "malformed ledger JSON emits continue, never aborts (fail-open, rc 0)" {
    # observe is unobservable so we reach the ledger path.
    _write_observe_mock_unobservable
    cat > "$TEST_DIR/autonomous-spend-ledger.sh" <<'SH'
#!/usr/bin/env bash
echo "not json at all {{{"
SH
    chmod +x "$TEST_DIR/autonomous-spend-ledger.sh"

    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=10000000 run_governor claude
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

# ── Regression: failed arm still parks and warns (safety-first) ───────────────

@test "arm failure still parks and warns on stderr" {
    _write_observe_mock_observable 90
    # usage-limit mock that fails.
    cat > "$TEST_DIR/autospec-usage-limit.sh" <<'SH'
#!/usr/bin/env bash
exit 1
SH
    chmod +x "$TEST_DIR/autospec-usage-limit.sh"

    run bash "$SCRIPT" claude \
        --repo-dir "$REPO_DIR" \
        --resume-command "autospec-autonomous resume" \
        --wait-seconds 3600
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^park '
    printf '%s\n' "$output" | grep -q 'WARNING: autospec-usage-limit.sh arm failed'
}

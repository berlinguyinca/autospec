#!/usr/bin/env bats
# tests/resume/test_resume_scan.bats — crash-resume decision engine.
#
# Covers (docs/specs/2026-06-03-crash-resume-design.md §Error handling):
#   - idempotency: resume writes NO run-state comment / adds NO lock
#   - pre-conditions: stop.flag / paused-by-user / all-closed / no-in-progress
#   - crash-vs-live: server updated_at age<300 not stolen; age>=threshold eligible
#   - cross-host: host != hostname / missing host under --resume-partial -> clean
#   - attempt cap: counter at max -> exit 0 cap-reached; resets on merged
#
# All GitHub/hostname access is via PATH-shadow mocks driven by env vars.

ROOT="${BATS_TEST_DIRNAME}/../.."
SCAN="$ROOT/skills/autospec-resume/scripts/resume-scan.sh"
ATTEMPTS="$ROOT/skills/autospec-resume/scripts/resume-attempts.sh"
REGISTRY="$ROOT/scripts/autospec-run-registry.sh"
HB_READ_REAL="$ROOT/skills/autospec-run/scripts/heartbeat-read.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$TEST_TMP/state"
    export AUTOSPEC_ACTIVE_RUNS_DIR="$TEST_TMP/active-runs"
    export AUTOSPEC_RESUME_ATTEMPTS_DIR="$TEST_TMP/resume-attempts"
    export AUTOSPEC_HEARTBEAT_DIR="$TEST_TMP/heartbeats"
    mkdir -p "$AUTOSPEC_STATE_DIR"
    export AUTOSPEC_HOST="host-A"

    # Point resume-scan at the Rust claim command mock plus the real heartbeat,
    # attempts, and registry helpers. Resume never re-implements claim state.
    export AUTOSPEC_HEARTBEAT_READ_SH="$HB_READ_REAL"
    export AUTOSPEC_RUN_REGISTRY_SH="$REGISTRY"
    export AUTOSPEC_RESUME_ATTEMPTS_SH="$ATTEMPTS"

    # Mock dir on PATH shadows gh and hostname; AUTOSPEC_BIN supplies the
    # durable Rust claim state reader.
    export MOCK_DIR="$TEST_TMP/bin"
    mkdir -p "$MOCK_DIR"
    export PATH="$MOCK_DIR:$PATH"

    # Defaults driving the gh mock. Tests override these.
    export GH_IN_PROGRESS="42"     # newline-separated issue numbers
    export GH_PAUSED=""            # newline-separated
    export GH_OPEN_AUTO="42"       # open auto-implement issues
    export GH_REPO="o/n"

    # Claim-state mock returns controlled JSON for `autospec claim state read`.
    export RS_STEP="claimed"
    export RS_STATE="claimed"
    export RS_UPDATED_AGE="600"    # seconds ago (server updated_at)
    export RS_BRANCH="feat/x"
    export RS_REPO="o/n"
    export RS_ISSUE="42"
    export RS_RAW=""
    export RECOVERY_RECOVERED="true"
    export RECOVERY_REPO="o/n"
    export RECOVERY_ISSUE="42"

    write_gh_mock
    write_hostname_mock
    write_claim_mock
}

teardown() { rm -rf "$TEST_TMP"; }

@test "resume scanner contains no explicit any usage" {
    run grep -En '(^|[^[:alnum:]_])any([^[:alnum:]_]|$)' "$SCAN"
    [ "$status" -eq 1 ]
}

write_hostname_mock() {
    cat > "$MOCK_DIR/hostname" <<EOF
#!/usr/bin/env bash
echo "${AUTOSPEC_HOST}"
EOF
    chmod +x "$MOCK_DIR/hostname"
}

write_gh_mock() {
    cat > "$MOCK_DIR/gh" <<'EOF'
#!/usr/bin/env bash
# PATH-shadow gh mock driven by env vars.
args="$*"
case "$args" in
    *"repo view"*) echo "${GH_REPO}"; exit 0 ;;
esac
case "$args" in
    *"issue list"*"--label in-progress-by-bot"*) printf '%s\n' $GH_IN_PROGRESS; exit 0 ;;
    *"issue list"*"--label paused-by-user"*)     printf '%s\n' $GH_PAUSED;      exit 0 ;;
    *"issue list"*"--label auto-implement"*)     printf '%s\n' $GH_OPEN_AUTO;   exit 0 ;;
    *"issue list"*)                              printf '%s\n' $GH_OPEN_AUTO;   exit 0 ;;
esac
# Any run-state comment write would land here -> record it as a violation.
case "$args" in
    *"issue comment"*) echo "VIOLATION: gh issue comment called" >>"${VIOLATION_LOG:-/dev/null}"; exit 0 ;;
    *"api"*) exit 0 ;;
esac
exit 0
EOF
    chmod +x "$MOCK_DIR/gh"
}

write_claim_mock() {
    local autospec="$MOCK_DIR/autospec"
    cat > "$autospec" <<'EOF'
#!/usr/bin/env bash
# Mocked Rust claim command: emits server `updated_at` state and owns the one
# permitted resume mutation, exact stale-startup recovery.
if [ "${1:-}" = "claim" ] && [ "${2:-}" = "state" ] && [ "${3:-}" = "read" ]; then
    if [ -n "${RS_RAW:-}" ]; then
        printf '%s\n' "$RS_RAW"
        exit 0
    fi
    updated="$(date -u -v-"${RS_UPDATED_AGE:-600}"S +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d "-${RS_UPDATED_AGE:-600} seconds" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null)"
    printf '{"schema":1,"repo":"%s","issue":%s,"step":"%s","state":"%s","branch":"%s","updated_at":"%s"}\n' \
            "${RS_REPO:-$GH_REPO}" "${RS_ISSUE:-42}" "${RS_STEP}" "${RS_STATE:-claimed}" "${RS_BRANCH}" "$updated"
    exit 0
fi
if [ "${1:-}" = "claim" ] && [ "${2:-}" = "state" ] && [ "${3:-}" = "recover-stale-startup" ]; then
    [ -z "${RECOVERY_LOG:-}" ] || printf 'RECOVER\n' >> "$RECOVERY_LOG"
    printf '{"recovered":%s,"issue":%s,"repo":"%s","reason":"test"}\n' \
        "${RECOVERY_RECOVERED:-true}" "${RECOVERY_ISSUE:-42}" "${RECOVERY_REPO:-$GH_REPO}"
    exit 0
fi
echo "VIOLATION: autospec $* (lock mutation) called" >>"${VIOLATION_LOG:-/dev/null}"
exit 1
EOF
    chmod +x "$autospec"
    export AUTOSPEC_BIN="$autospec"

    # A direct Rust caller must ignore this former shell-helper override. If
    # resume-scan still invokes it, the targeted red test cannot resume.
    local legacy="$MOCK_DIR/run-state-legacy-disallowed.sh"
    cat > "$legacy" <<'EOF'
#!/usr/bin/env bash
exit 91
EOF
    chmod +x "$legacy"
    export AUTOSPEC_RUN_STATE_SH="$legacy"
}

# Write a heartbeat for issue 42 with a given step + host.
write_heartbeat() {
    local issue="$1" step="$2" host="$3"
    local dir="$AUTOSPEC_HEARTBEAT_DIR/o_n"
    mkdir -p "$dir"
    printf '{"issue":"%s","branch":"feat/x","step":"%s","ts":%s,"pr":"","repo":"o/n","host":"%s"}\n' \
        "$issue" "$step" "$(date -u +%s)" "$host" > "$dir/$issue.json"
}

# Heartbeat WITHOUT a host field (backward-compat / legacy).
write_heartbeat_nohost() {
    local issue="$1" step="$2"
    local dir="$AUTOSPEC_HEARTBEAT_DIR/o_n"
    mkdir -p "$dir"
    printf '{"issue":"%s","branch":"feat/x","step":"%s","ts":%s,"pr":"","repo":"o/n"}\n' \
        "$issue" "$step" "$(date -u +%s)" > "$dir/$issue.json"
}

# Register a durable resume command so a relaunch is possible.
register_command() {
    bash "$REGISTRY" write --repo o/n --repo-dir /tmp/x --harness claude \
        --command "${1:-echo RELAUNCHED}" --host host-A >/dev/null
}

# ── Pre-conditions ────────────────────────────────────────────────────────────

@test "stop.flag present -> exit 0, no relaunch" {
    touch "$AUTOSPEC_STATE_DIR/stop.flag"
    write_heartbeat 42 claimed host-A
    register_command
    run bash "$SCAN" --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" == *"stop.flag present"* ]]
    [[ "$output" != *"resuming"* ]]
}

@test "paused-by-user present -> exit 0, no relaunch" {
    export GH_PAUSED="7"
    write_heartbeat 42 claimed host-A
    register_command
    run bash "$SCAN" --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" == *"paused-by-user present"* ]]
    [[ "$output" != *"resuming"* ]]
}

@test "all issues closed -> exit 0, no relaunch" {
    export GH_IN_PROGRESS=""
    export GH_OPEN_AUTO=""
    register_command
    run bash "$SCAN" --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" == *"all issues closed"* ]]
    [[ "$output" != *"resuming"* ]]
}

@test "no in-progress run-state -> exit 0, no relaunch" {
    export GH_IN_PROGRESS=""
    export GH_OPEN_AUTO="99"   # open issues exist but none in-progress
    register_command
    run bash "$SCAN" --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" == *"no open in-progress run-state"* ]]
    [[ "$output" != *"resuming"* ]]
}

# ── Crash-vs-live (server updated_at) ─────────────────────────────────────────

@test "claimed age < 300 -> not stolen (live/slow worker)" {
    export RS_STEP="claimed"
    export RS_UPDATED_AGE="120"   # < 300
    write_heartbeat 42 claimed host-A
    register_command
    run bash "$SCAN" --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" == *"not stolen"* ]] || [[ "$output" == *"live/slow"* ]]
    [[ "$output" != *"resuming"* ]]
}

@test "claimed age >= 300 -> eligible, resumes" {
    export RS_STEP="claimed"
    export RS_UPDATED_AGE="600"   # >= 300
    write_heartbeat 42 claimed host-A
    register_command "echo RELAUNCHED"
    run bash "$SCAN" --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" == *"resuming o/n"* ]]
    [[ "$output" == *"RELAUNCHED"* ]]
}

@test "non-claimed age >= 10800 -> eligible, resumes" {
    export RS_STEP="implementing"
    export RS_UPDATED_AGE="11000"   # >= 10800
    write_heartbeat 42 implementing host-A
    register_command "echo RELAUNCHED"
    run bash "$SCAN" --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" == *"resuming o/n"* ]]
}

@test "non-claimed age < 10800 -> not stolen" {
    export RS_STEP="implementing"
    export RS_UPDATED_AGE="500"   # < 10800 and step != claimed
    write_heartbeat 42 implementing host-A
    register_command
    run bash "$SCAN" --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" != *"resuming"* ]]
}

# ── Failed startup before label swap ─────────────────────────────────────────

@test "stale heartbeat-pending:none claim on auto-implement recovers before relaunch" {
    export GH_IN_PROGRESS=""
    export GH_OPEN_AUTO="42"
    export RS_STEP="heartbeat-pending:none"
    export RS_UPDATED_AGE="600"
    export RECOVERY_LOG="$TEST_TMP/events.log"
    register_command "printf 'RELAUNCH\\n' >>'$RECOVERY_LOG'"

    run bash "$SCAN" --repo o/n

    [ "$status" -eq 0 ]
    [ "$(cat "$RECOVERY_LOG")" = $'RECOVER\nRELAUNCH' ]
    [[ "$output" == *"resuming o/n"* ]]
}

@test "dry-run reports stale startup recovery and relaunch without mutation" {
    export GH_IN_PROGRESS=""
    export GH_OPEN_AUTO="42"
    export RS_STEP="heartbeat-pending:none"
    export RS_UPDATED_AGE="600"
    export RECOVERY_LOG="$TEST_TMP/events.log"
    register_command "printf 'RELAUNCH\\n' >>'$RECOVERY_LOG'"

    run bash "$SCAN" --repo o/n --dry-run

    [ "$status" -eq 0 ]
    [ ! -e "$RECOVERY_LOG" ]
    [[ "$output" == *"would recover"* ]]
    [[ "$output" == *"would resume"* ]]
}

@test "fresh heartbeat-pending:none claim is untouched" {
    export GH_IN_PROGRESS=""
    export GH_OPEN_AUTO="42"
    export RS_STEP="heartbeat-pending:none"
    export RS_UPDATED_AGE="120"
    export RECOVERY_LOG="$TEST_TMP/events.log"
    register_command "printf 'RELAUNCH\\n' >>'$RECOVERY_LOG'"

    run bash "$SCAN" --repo o/n

    [ "$status" -eq 0 ]
    [ ! -e "$RECOVERY_LOG" ]
    [[ "$output" != *"resuming"* ]]
}

@test "live heartbeat-pending:none claim refused by CAS recovery is not relaunched" {
    export GH_IN_PROGRESS=""
    export GH_OPEN_AUTO="42"
    export RS_STEP="heartbeat-pending:none"
    export RS_UPDATED_AGE="600"
    export RECOVERY_RECOVERED="false"
    export RECOVERY_LOG="$TEST_TMP/events.log"
    register_command "printf 'RELAUNCH\\n' >>'$RECOVERY_LOG'"

    run bash "$SCAN" --repo o/n

    [ "$status" -eq 0 ]
    [ "$(cat "$RECOVERY_LOG")" = "RECOVER" ]
    [[ "$output" != *"resuming"* ]]
}

@test "mismatched stale-startup recovery result is not relaunched" {
    export GH_IN_PROGRESS=""
    export GH_OPEN_AUTO="42"
    export RS_STEP="heartbeat-pending:none"
    export RS_UPDATED_AGE="600"
    export RECOVERY_REPO="other/repo"
    export RECOVERY_LOG="$TEST_TMP/events.log"
    register_command "printf 'RELAUNCH\\n' >>'$RECOVERY_LOG'"

    run bash "$SCAN" --repo o/n

    [ "$status" -eq 0 ]
    [ "$(cat "$RECOVERY_LOG")" = "RECOVER" ]
    [[ "$output" != *"resuming"* ]]
}

@test "malformed startup claim is untouched" {
    export GH_IN_PROGRESS=""
    export GH_OPEN_AUTO="42"
    export RS_RAW='{not-json'
    export RECOVERY_LOG="$TEST_TMP/events.log"
    register_command "printf 'RELAUNCH\\n' >>'$RECOVERY_LOG'"

    run bash "$SCAN" --repo o/n

    [ "$status" -eq 0 ]
    [ ! -e "$RECOVERY_LOG" ]
    [[ "$output" != *"resuming"* ]]
}

@test "mismatched startup claim identity is untouched" {
    export GH_IN_PROGRESS=""
    export GH_OPEN_AUTO="42"
    export RS_STEP="heartbeat-pending:none"
    export RS_UPDATED_AGE="600"
    export RS_REPO="other/repo"
    export RECOVERY_LOG="$TEST_TMP/events.log"
    register_command "printf 'RELAUNCH\\n' >>'$RECOVERY_LOG'"

    run bash "$SCAN" --repo o/n

    [ "$status" -eq 0 ]
    [ ! -e "$RECOVERY_LOG" ]
    [[ "$output" != *"resuming"* ]]
}

@test "startup claim with non-numeric issue identity is untouched" {
    export GH_IN_PROGRESS=""
    export GH_OPEN_AUTO="42"
    updated="$(date -u -v-600S +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d '-600 seconds' +'%Y-%m-%dT%H:%M:%SZ')"
    export RS_RAW="{\"state\":\"claimed\",\"step\":\"heartbeat-pending:none\",\"repo\":\"o/n\",\"issue\":\"42\",\"updated_at\":\"$updated\"}"
    export RECOVERY_LOG="$TEST_TMP/events.log"
    register_command "printf 'RELAUNCH\\n' >>'$RECOVERY_LOG'"

    run bash "$SCAN" --repo o/n

    [ "$status" -eq 0 ]
    [ ! -e "$RECOVERY_LOG" ]
    [[ "$output" != *"resuming"* ]]
}

@test "non-exact heartbeat-pending step is untouched" {
    export GH_IN_PROGRESS=""
    export GH_OPEN_AUTO="42"
    export RS_STEP="heartbeat-pending:worker-a"
    export RS_UPDATED_AGE="600"
    export RECOVERY_LOG="$TEST_TMP/events.log"
    register_command "printf 'RELAUNCH\\n' >>'$RECOVERY_LOG'"

    run bash "$SCAN" --repo o/n

    [ "$status" -eq 0 ]
    [ ! -e "$RECOVERY_LOG" ]
    [[ "$output" != *"resuming"* ]]
}

@test "terminal heartbeat step merged -> not resumed" {
    export RS_UPDATED_AGE="600"
    write_heartbeat 42 merged host-A
    register_command
    run bash "$SCAN" --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" != *"resuming"* ]]
}

# ── Idempotency / no second lock ──────────────────────────────────────────────

@test "idempotency: resume writes NO run-state comment / no lock mutation" {
    export VIOLATION_LOG="$TEST_TMP/violations.log"
    : > "$VIOLATION_LOG"
    export RS_UPDATED_AGE="600"
    write_heartbeat 42 claimed host-A
    register_command "echo RELAUNCHED"
    # Two concurrent resumes.
    bash "$SCAN" --repo o/n >/dev/null 2>&1 &
    bash "$SCAN" --repo o/n >/dev/null 2>&1 &
    wait
    # Neither resume may have written a run-state comment or mutated the lock.
    [ ! -s "$VIOLATION_LOG" ]
}

# ── Cross-host gate (--resume-partial) ────────────────────────────────────────

@test "cross-host: heartbeat host != hostname -> clean-restart, not partial" {
    export RS_UPDATED_AGE="600"
    write_heartbeat 42 claimed host-B   # different host than AUTOSPEC_HOST=host-A
    register_command "echo RELAUNCHED"
    run bash "$SCAN" --repo o/n --resume-partial
    [ "$status" -eq 0 ]
    [[ "$output" == *"resuming o/n"* ]]
    [[ "$output" == *"clean-restart off origin/main"* ]]
    [[ "$output" != *"resume-partial"* ]]
}

@test "cross-host: missing host -> clean-restart, not partial" {
    export RS_UPDATED_AGE="600"
    write_heartbeat_nohost 42 claimed
    register_command "echo RELAUNCHED"
    run bash "$SCAN" --repo o/n --resume-partial
    [ "$status" -eq 0 ]
    [[ "$output" == *"clean-restart off origin/main"* ]]
    [[ "$output" != *"resume-partial"* ]]
}

@test "same-host --resume-partial -> partial re-attach" {
    export RS_UPDATED_AGE="600"
    write_heartbeat 42 claimed host-A
    register_command "echo RELAUNCHED"
    run bash "$SCAN" --repo o/n --resume-partial
    [ "$status" -eq 0 ]
    [[ "$output" == *"resume-partial /tmp/wt-feat/x"* ]]
}

# ── Attempt cap ───────────────────────────────────────────────────────────────

@test "attempt cap reached -> exit 0 cap-reached, no relaunch" {
    export AUTOSPEC_RESUME_MAX_ATTEMPTS=3
    export RS_UPDATED_AGE="600"
    write_heartbeat 42 claimed host-A
    register_command "echo RELAUNCHED"
    # Drive the counter to the cap.
    bash "$ATTEMPTS" inc --repo o/n >/dev/null
    bash "$ATTEMPTS" inc --repo o/n >/dev/null
    bash "$ATTEMPTS" inc --repo o/n >/dev/null
    run bash "$SCAN" --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" == *"attempt cap reached"* ]]
    [[ "$output" != *"resuming"* ]]
}

@test "attempt counter resets on merged (forward progress)" {
    export AUTOSPEC_RESUME_MAX_ATTEMPTS=3
    bash "$ATTEMPTS" inc --repo o/n >/dev/null
    bash "$ATTEMPTS" inc --repo o/n >/dev/null
    bash "$ATTEMPTS" inc --repo o/n >/dev/null
    run bash "$ATTEMPTS" at-cap --repo o/n
    [ "$status" -eq 0 ]   # at cap
    # A merged issue resets the counter.
    bash "$ATTEMPTS" reset --repo o/n >/dev/null
    run bash "$ATTEMPTS" at-cap --repo o/n
    [ "$status" -eq 1 ]   # no longer at cap
    run bash "$ATTEMPTS" get --repo o/n
    [ "$output" = "0" ]
}

@test "resume increments the attempt counter on relaunch" {
    export RS_UPDATED_AGE="600"
    write_heartbeat 42 claimed host-A
    register_command "echo RELAUNCHED"
    run bash "$ATTEMPTS" get --repo o/n
    [ "$output" = "0" ]
    bash "$SCAN" --repo o/n >/dev/null 2>&1
    run bash "$ATTEMPTS" get --repo o/n
    [ "$output" = "1" ]
}

# ── --dry-run ─────────────────────────────────────────────────────────────────

@test "--dry-run prints decision, changes nothing, exit 0" {
    export RS_UPDATED_AGE="600"
    write_heartbeat 42 claimed host-A
    register_command "echo RELAUNCHED"
    run bash "$SCAN" --repo o/n --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"[dry-run]"* ]]
    [[ "$output" == *"command=echo RELAUNCHED"* ]]
    # No attempt increment under dry-run.
    run bash "$ATTEMPTS" get --repo o/n
    [ "$output" = "0" ]
}

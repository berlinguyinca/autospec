#!/usr/bin/env bash
# Regression coverage for issue #1730. This file intentionally runs under
# `bash tests/unit/autospec-autonomous-infer-default.bats` because the issue's
# primary smoke command invokes bash directly.

set -u

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"
PERSONA_SOURCES="$REPO_ROOT/scripts/autonomous-persona-sources.sh"
FAILURES=0

fail() {
    printf 'not ok - %s\n' "$1" >&2
    FAILURES=$((FAILURES + 1))
}

pass() {
    printf 'ok - %s\n' "$1"
}

_helper() {
    local name="$1"
    local body="$2"
    printf '#!/usr/bin/env bash\n%s\n' "$body" > "$SCRIPTS_DIR/$name"
    chmod +x "$SCRIPTS_DIR/$name"
}

setup_case() {
    TMP="$(mktemp -d -t test-autonomous-infer-default.XXXXXX)"
    FAKE_REPO="$TMP/repo"
    SCRIPTS_DIR="$FAKE_REPO/scripts"
    HOME_DIR="$TMP/home"
    mkdir -p "$SCRIPTS_DIR" "$FAKE_REPO/.autospec" "$HOME_DIR/.autospec"

    EXPLORE_LOG="$TMP/explore.log"
    BOOTSTRAP_LOG="$TMP/bootstrap.log"
    INTERVIEW_LOG="$TMP/interview.log"
    SYNTH_LOG="$TMP/synth.log"
    touch "$EXPLORE_LOG" "$BOOTSTRAP_LOG" "$INTERVIEW_LOG" "$SYNTH_LOG"

    _helper autonomous-control-channel.sh 'exit 0'
    _helper autonomous-premerge-gate.sh 'printf "merge-ok\n"'
    _helper autonomous-spend-ledger.sh 'case "${1:-}" in add) exit 0;; check) printf "continue\n";; *) exit 0;; esac'
    _helper autonomous-resilience.sh 'case "${1:-}" in state) printf "DECISION:state-written\n";; lock) printf "DECISION:lock-acquired\nLOCK_SESSION:test\n";; *) exit 0;; esac'
    _helper autospec-usage-limit.sh 'exit 0'
    _helper autonomous-waterfall.sh 'printf '\''{"tier":2,"action":"run-explore-once","reason":"test-tier-2"}\n'\'''
    _helper autospec 'case "${1:-} ${2:-}" in "queue ready") printf '\''{"ready":[],"blocked":[],"claimed":[],"conflicts":[],"worker_cap":{"reached":false},"batch":[]}\n'\'';; *) exit 0;; esac'
    cat > "$SCRIPTS_DIR/autonomous-persona-synth.sh" <<EOF_SYNTH
#!/usr/bin/env bash
printf 'synth-called %s\n' "\$*" >> '$SYNTH_LOG'
exit 0
EOF_SYNTH
    chmod +x "$SCRIPTS_DIR/autonomous-persona-synth.sh"
}

teardown_case() {
    rm -rf "${TMP:-}"
}

run_conductor_once() {
    local extra_env="${1:-}"
    OUTPUT="$({
        set -eu
        . "$LOOP_LIB"
        HOME="$HOME_DIR" \
        CONDUCTOR_SCRIPTS_DIR="$SCRIPTS_DIR" \
        CONDUCTOR_REPO='test-owner/test-repo' \
        CONDUCTOR_MAX_CYCLES=1 \
        CONDUCTOR_POLL_INTERVAL=0 \
        CONDUCTOR_DRY_RUN=0 \
        CONDUCTOR_NO_DIGEST=1 \
        AUTOSPEC_QUEUE_BIN="$SCRIPTS_DIR/autospec" \
        AUTOSPEC_PERSONA_SOURCES_CMD="$PERSONA_SOURCES" \
        AUTOSPEC_EXPLORE_CMD="printf 'explore-called\\n' >> '$EXPLORE_LOG'; printf '{\"dry\":false,\"filed\":1}\\n'" \
        AUTOSPEC_RUN_CMD="printf 'drain-called\\n' >> '$EXPLORE_LOG'" \
        AUTOSPEC_BOOTSTRAP_DECISION_CMD="printf 'bootstrap-called\\n' >> '$BOOTSTRAP_LOG'" \
        AUTOSPEC_BOOTSTRAP_INTERVIEW_CMD="printf 'interview-called\\n' >> '$INTERVIEW_LOG'" \
        eval "$extra_env autospec_conductor_run"
    } 2>&1)"
    STATUS=$?
}

assert_status_zero() { [ "$STATUS" -eq 0 ] || { printf '%s\n' "$OUTPUT" >&2; return 1; }; }
assert_file_contains() { grep -q "$1" "$2"; }
assert_file_not_contains() { ! grep -q "$1" "$2"; }
assert_output_contains() { printf '%s\n' "$OUTPUT" | grep -q "$1"; }
assert_output_not_contains() { ! printf '%s\n' "$OUTPUT" | grep -q "$1"; }

run_case() {
    local name="$1"
    shift
    setup_case
    if "$@"; then
        pass "$name"
    else
        fail "$name"
        printf '%s\n' "--- output ---" >&2
        printf '%s\n' "${OUTPUT:-}" >&2
        printf '%s\n' "--- explore log ---" >&2
        cat "${EXPLORE_LOG:-/dev/null}" >&2 2>/dev/null || true
        printf '%s\n' "--- bootstrap log ---" >&2
        cat "${BOOTSTRAP_LOG:-/dev/null}" >&2 2>/dev/null || true
        printf '%s\n' "--- interview log ---" >&2
        cat "${INTERVIEW_LOG:-/dev/null}" >&2 2>/dev/null || true
    fi
    teardown_case
}

case_non_empty_headless_proceeds() {
    printf '# repo operating context\n' > "$FAKE_REPO/AGENTS.md"
    run_conductor_once
    assert_status_zero && \
    assert_file_contains 'explore-called' "$EXPLORE_LOG" && \
    assert_output_not_contains 'no-steering' && \
    assert_file_not_contains 'bootstrap-called' "$BOOTSTRAP_LOG"
}

case_empty_headless_bootstrap_parks() {
    run_conductor_once
    assert_status_zero && \
    assert_file_contains 'bootstrap-called' "$BOOTSTRAP_LOG" && \
    assert_file_not_contains 'explore-called' "$EXPLORE_LOG" && \
    assert_output_contains 'bootstrap' && \
    assert_output_not_contains 'no-steering'
}

case_empty_interactive_interview() {
    run_conductor_once 'AUTOSPEC_BOOTSTRAP_INTERACTIVE=1'
    assert_status_zero && \
    assert_file_contains 'interview-called' "$INTERVIEW_LOG" && \
    assert_file_not_contains 'bootstrap-called' "$BOOTSTRAP_LOG" && \
    assert_file_not_contains 'explore-called' "$EXPLORE_LOG"
}

case_low_confidence_digest() {
    printf '# repo operating context\n' > "$FAKE_REPO/AGENTS.md"
    run_conductor_once 'CONDUCTOR_NO_DIGEST=0'
    assert_status_zero && \
    assert_file_contains 'explore-called' "$EXPLORE_LOG" && \
    assert_file_contains 'low-confidence' "$FAKE_REPO/.autospec/autonomous-digest.md"
}

case_absent_priorities_not_park() {
    printf '# repo operating context\n' > "$FAKE_REPO/AGENTS.md"
    [ ! -f "$HOME_DIR/.autospec/autonomous-priorities.md" ] || return 1
    run_conductor_once
    assert_status_zero && \
    assert_file_contains 'explore-called' "$EXPLORE_LOG" && \
    assert_output_not_contains 'parking: no-steering'
}

run_case 'non-empty inferred bundle in headless mode proceeds steered without parking' case_non_empty_headless_proceeds
run_case 'empty bundle in headless mode files bootstrap decision and parks only that decision' case_empty_headless_bootstrap_parks
run_case 'empty bundle in interactive mode runs bootstrap interview dialog' case_empty_interactive_interview
run_case 'non-empty low-confidence bundle proceeds and flags confidence in digest' case_low_confidence_digest
run_case 'absence of priorities file alone does not park when inference signals exist' case_absent_priorities_not_park

if [ "$FAILURES" -ne 0 ]; then
    printf '%s test(s) failed\n' "$FAILURES" >&2
    exit 1
fi

#!/usr/bin/env bash
# scripts/autonomous-usage-governor.sh — soft-park at AUTOSPEC_USAGE_SOFT_PCT (default 90).
#
# Checks whether the conductor should soft-park BEFORE the hard-quota backstop
# fires.  Two signal sources are tried in order:
#
#   1. Live fraction: usage-observe.sh <harness> — if observable:true, compares
#      .percent to AUTOSPEC_USAGE_SOFT_PCT.
#   2. Spend-ledger tally: autonomous-spend-ledger.sh status — compares .tokens
#      to AUTOSPEC_USAGE_SOFT_PCT % of AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS.
#
# Stdout: "continue" | "park <resume-at>"   (exit code always 0)
#
# Usage:
#   autonomous-usage-governor.sh <harness> [--repo-dir DIR]
#                                [--resume-command CMD]
#                                [--resume-at ISO8601 | --wait-seconds N]
#                                [--run-id ID]
#
# Environment:
#   AUTOSPEC_USAGE_SOFT_PCT              soft-park threshold percent (default 90)
#   AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS  lifetime token cap (default 10 000 000)
#
# This script layers on top of the existing hard-quota backstop
# (autonomous-spend-ledger.sh check) — it does NOT replace or modify it.
#
# Conventions: set -eu; if/then/fi one-sided conditionals
# (feedback_bash_set_e_short_circuit); no RETURN traps
# (feedback_bash_return_trap_leak).

set -eu

PROG="$(basename "$0")"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

DEFAULT_SOFT_PCT=90
DEFAULT_LIFETIME_TOKENS=10000000
DEFAULT_WAIT_SECONDS=3600

die() {
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit 2
}

info() {
    printf '[autonomous-usage-governor] %s\n' "$*" >&2
}

require_jq() {
    command -v jq >/dev/null 2>&1 || die "jq is required"
}

iso_future() {
    wait_secs="$1"
    epoch="$(date -u +%s)"
    future="$(( epoch + wait_secs ))"
    # macOS-compatible then Linux fallback.
    if date -u -r "$future" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null; then
        return 0
    fi
    date -u -d "@$future" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || printf '%s' "$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
}

# Locate a helper script: PATH first → alongside this script → ~/.autospec/scripts/.
# Prints the path (or empty string when not found). Always exits 0.
find_script() {
    script_name="$1"
    if command -v "$script_name" >/dev/null 2>&1; then
        printf '%s' "$(command -v "$script_name")"
        return 0
    fi
    local_path="${SCRIPT_DIR}/${script_name}"
    if [ -f "$local_path" ]; then
        printf '%s' "$local_path"
        return 0
    fi
    installed="${HOME}/.autospec/scripts/${script_name}"
    if [ -f "$installed" ]; then
        printf '%s' "$installed"
        return 0
    fi
    printf ''
    # Not found; caller checks for empty string.
}

# Find notify.sh (mirrors autonomous-spend-ledger.sh pattern).
find_notify() {
    repo_dir="${1:-$(pwd)}"
    if command -v notify.sh >/dev/null 2>&1; then
        printf 'notify.sh'
        return 0
    fi
    skill_path="${repo_dir}/skills/autospec-shared/scripts/notify.sh"
    if [ -f "$skill_path" ]; then
        printf '%s' "$skill_path"
        return 0
    fi
    installed="${HOME}/.autospec/skills/autospec-shared/scripts/notify.sh"
    if [ -f "$installed" ]; then
        printf '%s' "$installed"
        return 0
    fi
    printf ''
}

# True (exit 0) when val is a number in [0,100] with at least one leading digit.
is_valid_percent() {
    val="$1"
    printf '%s' "$val" | grep -Eq '^[0-9]+(\.[0-9]+)?$' || return 1
    awk -v v="$val" 'BEGIN { exit !(v >= 0 && v <= 100) }'
}

# True (exit 0) when val is a non-negative integer string.
is_valid_integer() {
    val="$1"
    case "$val" in
        ''|*[!0-9]*) return 1 ;;
    esac
    return 0
}

# ── Argument parsing ──────────────────────────────────────────────────────────

HARNESS="${1:-}"
case "$HARNESS" in
    --help|-h)
        cat <<EOF
Usage: $PROG <harness> [options]

  <harness>             one of: claude | codex | opencode

Options:
  --repo-dir DIR        repository root (default: cwd)
  --resume-command CMD  command to pass to autospec-usage-limit.sh arm
  --resume-at ISO8601   UTC time to arm for; mutually exclusive with --wait-seconds
  --wait-seconds N      seconds until resume (default: ${DEFAULT_WAIT_SECONDS})
  --run-id ID           run-id forwarded to autospec-usage-limit.sh arm

Environment:
  AUTOSPEC_USAGE_SOFT_PCT              (default ${DEFAULT_SOFT_PCT})
  AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS  (default ${DEFAULT_LIFETIME_TOKENS})

Stdout: "continue" | "park <resume-at>"   (exit code always 0)
EOF
        exit 0
        ;;
    '')
        die "missing <harness> argument (expected: claude | codex | opencode)"
        ;;
    claude|codex|opencode)
        ;;
    *)
        die "unknown harness '${HARNESS}' (expected: claude | codex | opencode)"
        ;;
esac
shift

REPO_DIR="$(pwd)"
RESUME_COMMAND=""
RESUME_AT=""
WAIT_SECONDS=""
RUN_ID=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-dir)       REPO_DIR="${2:-}";       shift 2 ;;
        --resume-command) RESUME_COMMAND="${2:-}";  shift 2 ;;
        --resume-at)      RESUME_AT="${2:-}";       shift 2 ;;
        --wait-seconds)   WAIT_SECONDS="${2:-}";    shift 2 ;;
        --run-id)         RUN_ID="${2:-}";          shift 2 ;;
        --help|-h)        "$0" --help; exit 0 ;;
        *)                die "unknown option: $1" ;;
    esac
done

# Resolve and validate config from env.
SOFT_PCT="${AUTOSPEC_USAGE_SOFT_PCT:-$DEFAULT_SOFT_PCT}"
LIFETIME_TOKENS="${AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS:-$DEFAULT_LIFETIME_TOKENS}"

if ! is_valid_percent "$SOFT_PCT"; then
    info "AUTOSPEC_USAGE_SOFT_PCT='${SOFT_PCT}' is not a valid percent; using default ${DEFAULT_SOFT_PCT}"
    SOFT_PCT="$DEFAULT_SOFT_PCT"
fi

if ! is_valid_integer "$LIFETIME_TOKENS"; then
    info "AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS='${LIFETIME_TOKENS}' is not a valid integer; using default ${DEFAULT_LIFETIME_TOKENS}"
    LIFETIME_TOKENS="$DEFAULT_LIFETIME_TOKENS"
fi

# Compute resume-at timestamp once (used only on park).
if [ -n "$RESUME_AT" ] && [ -n "$WAIT_SECONDS" ]; then
    die "--resume-at and --wait-seconds are mutually exclusive"
fi
if [ -z "$RESUME_AT" ]; then
    wait_for="${WAIT_SECONDS:-$DEFAULT_WAIT_SECONDS}"
    if ! is_valid_integer "$wait_for"; then
        wait_for="$DEFAULT_WAIT_SECONDS"
    fi
    RESUME_AT="$(iso_future "$wait_for")"
fi

# ── Soft-park action ──────────────────────────────────────────────────────────

do_soft_park() {
    park_reason="$1"
    info "soft-park triggered: ${park_reason} (threshold: ${SOFT_PCT}%)"

    # Call notify.sh — fail-open: notifier errors must not block the park verdict.
    notifier="$(find_notify "$REPO_DIR")"
    if [ -n "$notifier" ]; then
        bash "$notifier" "autospec-autonomous soft-park" "$park_reason" || true
    fi

    # Arm autospec-usage-limit.sh if a resume command was supplied.
    if [ -n "$RESUME_COMMAND" ]; then
        limit_sh="$(find_script "autospec-usage-limit.sh")"
        if [ -n "$limit_sh" ]; then
            if [ -n "$RUN_ID" ]; then
                bash "$limit_sh" arm \
                    --harness "$HARNESS" \
                    --repo-dir "$REPO_DIR" \
                    --command "$RESUME_COMMAND" \
                    --resume-at "$RESUME_AT" \
                    --run-id "$RUN_ID" || true
            else
                bash "$limit_sh" arm \
                    --harness "$HARNESS" \
                    --repo-dir "$REPO_DIR" \
                    --command "$RESUME_COMMAND" \
                    --resume-at "$RESUME_AT" || true
            fi
        else
            info "autospec-usage-limit.sh not found; skipping arm"
        fi
    fi

    printf 'park %s\n' "$RESUME_AT"
}

# ── Signal source 1: live observable fraction via usage-observe.sh ────────────

require_jq

observe_sh="$(find_script "usage-observe.sh")"
if [ -n "$observe_sh" ]; then
    observe_json=""
    observe_rc=0
    observe_json="$(bash "$observe_sh" "$HARNESS" 2>/dev/null)" || observe_rc=$?

    if [ "$observe_rc" -eq 0 ] && [ -n "$observe_json" ]; then
        observable="$(printf '%s' "$observe_json" | jq -r '.observable // "false"')"
        if [ "$observable" = "true" ]; then
            live_pct="$(printf '%s' "$observe_json" | jq -r '.percent // "null"')"
            if is_valid_percent "$live_pct"; then
                # Compare live_pct to SOFT_PCT using awk (handles decimals; avoids
                # shell integer overflow; exit 0 = at/above threshold).
                if awk -v pct="$live_pct" -v threshold="$SOFT_PCT" \
                        'BEGIN { exit !(pct >= threshold) }'; then
                    do_soft_park "live usage ${live_pct}% >= soft threshold ${SOFT_PCT}%"
                    exit 0
                fi
                printf 'continue\n'
                exit 0
            fi
        fi
        # observable=false or invalid percent — fall through to ledger tally.
    fi
fi

# ── Signal source 2: spend-ledger token tally (fallback) ─────────────────────

ledger_sh="$(find_script "autonomous-spend-ledger.sh")"
if [ -n "$ledger_sh" ]; then
    ledger_json=""
    ledger_rc=0
    ledger_json="$(bash "$ledger_sh" status --repo-dir "$REPO_DIR" 2>/dev/null)" || ledger_rc=$?

    if [ "$ledger_rc" -eq 0 ] && [ -n "$ledger_json" ]; then
        current_tokens="$(printf '%s' "$ledger_json" | jq -r '.tokens // 0')"
        if ! is_valid_integer "$current_tokens"; then
            current_tokens=0
        fi

        # soft_threshold = floor(LIFETIME_TOKENS * SOFT_PCT / 100).
        # Uses awk to handle large numbers without shell integer overflow.
        soft_threshold="$(awk -v lt="$LIFETIME_TOKENS" -v pct="$SOFT_PCT" \
            'BEGIN { printf "%d", lt * pct / 100 }')"

        # A soft_threshold of 0 means the cap is effectively disabled (e.g.
        # LIFETIME_TOKENS=0 disables the token cap).
        if [ "$soft_threshold" -gt 0 ] && [ "$current_tokens" -ge "$soft_threshold" ]; then
            do_soft_park "spend-ledger tokens ${current_tokens} >= soft threshold ${soft_threshold} (${SOFT_PCT}% of ${LIFETIME_TOKENS})"
            exit 0
        fi
    fi
fi

printf 'continue\n'
exit 0

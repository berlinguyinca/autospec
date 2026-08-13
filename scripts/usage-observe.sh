#!/usr/bin/env bash
# scripts/usage-observe.sh — per-harness live-usage observability probe (F6a spike).
#
# Emits a single JSON object reporting whether a LIVE usage fraction (percent of
# quota consumed this session) is observable for the given harness:
#
#   {"harness":"<h>","observable":<bool>,"percent":<0-100|null>,"source":"<why>"}
#
# F6a SPIKE FINDING (2026-06-26): none of the three supported harnesses expose a
# deterministic live usage fraction today, so the default for every harness is
#   observable=false, percent=null
# and the autospec-autonomous governor (F6b) MUST fall back to the existing
# spend-ledger token tally (autonomous-spend-ledger.sh) at 90% of
# AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS. Per-harness rationale:
#
#   claude   — Claude Code exposes no env var or session signal carrying a live
#              quota fraction. Per-message token counts live in the session
#              transcript (~/.claude/projects/.../*.jsonl) but that is a
#              cumulative tally, not a quota percentage — i.e. the spend-ledger
#              fallback, not a live fraction.
#   codex    — The Codex CLI surfaces no session-level quota fraction. The
#              Anthropic/OpenAI rate-limit headers are per-request and reset-based,
#              not a cumulative session percentage the loop can poll.
#   opencode — OpenCode routes to an operator-chosen provider, so there is no
#              unified usage signal; whatever the provider returns is per-request
#              and not normalized into a session fraction.
#
# PROBE SEAM (forward-compatible, honest default): if a harness ever ships a live
# fraction, wire it without editing this script by setting the per-harness env var
#   AUTOSPEC_USAGE_PROBE_CLAUDE / _CODEX / _OPENCODE
# to an executable command that prints a single number 0-100 (the live percent)
# on stdout and exits 0. When set and it yields a valid number, this script
# reports observable=true with that percent. When unset, failing, or non-numeric,
# it reports the honest observable=false default above. (This same seam is what
# the bats suite mocks as a subprocess.)
#
# Conventions: set -eu; if/then/fi one-sided conditionals
# (feedback_bash_set_e_short_circuit); no RETURN traps
# (feedback_bash_return_trap_leak).

set -eu

PROG="$(basename "$0")"

die() {
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit 2
}

usage() {
    cat <<EOF
Usage: $PROG <harness>

  <harness>   one of: claude | codex | opencode

Emits a JSON object: {harness, observable, percent, source}.
An unknown harness exits non-zero.

Probe seam: set AUTOSPEC_USAGE_PROBE_CLAUDE / _CODEX / _OPENCODE to an
executable that prints a live percent (0-100) to override the default
observable=false finding for that harness.
EOF
}

# Honest per-harness default rationale when no live fraction is observable.
default_source_for() {
    harness="$1"
    case "$harness" in
        claude)
            printf 'no live quota fraction exposed by Claude Code (per-message token counts in the session transcript are a cumulative tally, not a quota %%); fall back to spend-ledger token tally at AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS'
            ;;
        codex)
            printf 'no live quota fraction exposed by Codex CLI (rate-limit headers are per-request/reset-based, not a session %%); fall back to spend-ledger token tally'
            ;;
        opencode)
            printf 'no live quota fraction exposed by OpenCode (provider-dependent, no unified session signal); fall back to spend-ledger token tally'
            ;;
        *)
            printf 'unknown harness'
            ;;
    esac
}

# True (exit 0) when val is a number in [0,100]. Accepts integers and decimals.
# Requires at least one leading digit so bare/partial dots ("." / "1." / ".5" /
# "1.2.3") are rejected before the numeric range check (a bare "." would coerce
# to 0 in awk and then break `jq --argjson`).
is_valid_percent() {
    val="$1"
    printf '%s' "$val" | grep -Eq '^[0-9]+(\.[0-9]+)?$' || return 1
    awk -v v="$val" 'BEGIN { exit !(v >= 0 && v <= 100) }'
}

emit_json() {
    harness="$1"
    observable="$2"   # literal: true | false
    percent="$3"      # literal: null | a number
    source="$4"
    if command -v jq >/dev/null 2>&1; then
        jq -n \
            --arg h "$harness" \
            --argjson o "$observable" \
            --argjson p "$percent" \
            --arg s "$source" \
            '{harness: $h, observable: $o, percent: $p, source: $s}'
    else
        # jq-less fallback: source is a controlled string (no embedded quotes).
        printf '{"harness":"%s","observable":%s,"percent":%s,"source":"%s"}\n' \
            "$harness" "$observable" "$percent" "$source"
    fi
}

probe_harness() {
    harness="$1"
    var="AUTOSPEC_USAGE_PROBE_$(printf '%s' "$harness" | tr '[:lower:]' '[:upper:]')"
    # bash 3.2 indirect expansion; empty when the override is unset.
    cmd="${!var:-}"

    # Auto-discover the shipped OpenCode probe (reads the OpenCode SQLite DB for
    # a trailing-window token tally) when the operator has not set an explicit
    # probe. Other harnesses keep the honest observable=false default.
    if [ -z "$cmd" ] && [ "$harness" = "opencode" ]; then
        script_dir="$(CDPATH= cd -- "$(dirname "$0")" 2>/dev/null && pwd)"
        for candidate in \
            "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lib/opencode-usage-probe.sh" \
            "$script_dir/lib/opencode-usage-probe.sh"; do
            if [ -x "$candidate" ]; then
                cmd="$candidate"
                break
            fi
        done
    fi

    if [ -n "$cmd" ]; then
        probe_raw=""
        probe_rc=0
        # Capture the probe command's OWN exit status (no pipeline in the
        # command substitution, so a probe that prints then exits non-zero is
        # correctly rejected rather than masked by head/tr's exit status).
        probe_raw="$($cmd 2>/dev/null)" || probe_rc=$?
        # First line, trimming leading/trailing whitespace only (internal
        # whitespace stays so "1 2" is rejected by is_valid_percent).
        probe_out="$(printf '%s\n' "$probe_raw" | head -1 \
            | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')"
        if [ "$probe_rc" = "0" ] && is_valid_percent "$probe_out"; then
            emit_json "$harness" true "$probe_out" "probe:${var}"
            return 0
        fi
        # Probe set but unusable → fall through to the honest default.
    fi

    emit_json "$harness" false null "$(default_source_for "$harness")"
}

main() {
    harness="${1:-}"
    case "$harness" in
        -h|--help)
            usage
            exit 0
            ;;
        '')
            usage >&2
            die "missing <harness> argument"
            ;;
        claude|codex|opencode)
            probe_harness "$harness"
            ;;
        *)
            die "unknown harness '$harness' (expected: claude | codex | opencode)"
            ;;
    esac
}

main "$@"

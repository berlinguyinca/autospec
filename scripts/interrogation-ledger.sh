#!/usr/bin/env bash
# scripts/interrogation-ledger.sh — decision ledger for the reuse-interrogation lens.
#
# Records per-verdict outcomes and computes per-trigger precision to support
# auto-demotion of noisy triggers below the precision floor.
#
# Whole feature gated by AUTOSPEC_REUSE_LENS (default OFF). When unset or not
# equal to "1", all subcommands exit 0 immediately — byte-identical behavior to
# today (inert). Assert this in tests.
#
# Ledger path: .autospec/interrogation-ledger.jsonl (one JSON object per line):
#   {"ts":<unix>,"issue":"<N>","pr":"<N>","trigger":"REINVENT_REPO_UTIL","verdict":"BLOCK","upheld":true}
#
# Env overrides (for test isolation):
#   AUTOSPEC_REUSE_LENS              set to "1" to arm; default OFF (inert).
#   AUTOSPEC_REUSE_PRECISION_FLOOR   default 0.6
#   AUTOSPEC_REUSE_DEMOTE_AFTER      default 10
#   AUTOSPEC_INTERROGATION_LEDGER    explicit ledger path (overrides auto-derived default)
#
# Subcommands:
#   record --issue N --pr N --trigger T --verdict V [--upheld true|false|null]
#       Append one JSONL line to the ledger.
#       Write failure → warn and exit 0 (best-effort; never blocks the PR).
#
#   report [--ledger PATH] [--floor F] [--after N]
#       Print per-trigger precision table. Triggers below floor after >=N runs
#       are flagged "DEMOTE" (auto-demote to ADVISE).
#
#   precision [--ledger PATH] [--floor F] [--after N]
#       Same as report, but exits non-zero when any trigger should be demoted.
#
# Bash rules: set -eu; if/then/fi for one-sided conditionals (no `[ ] &&`);
# no RETURN traps; jq capture()/== not dynamic test() for user-derived values
# (injection guard per feedback_jq_test_regex_metachar_injection).

set -eu

PROG="interrogation-ledger.sh"

DEFAULT_PRECISION_FLOOR="0.6"
DEFAULT_DEMOTE_AFTER="10"

PRECISION_FLOOR="${AUTOSPEC_REUSE_PRECISION_FLOOR:-$DEFAULT_PRECISION_FLOOR}"
DEMOTE_AFTER="${AUTOSPEC_REUSE_DEMOTE_AFTER:-$DEFAULT_DEMOTE_AFTER}"

usage() {
    cat <<'EOF'
Usage:
  interrogation-ledger.sh record --issue N --pr N --trigger T --verdict V [--upheld true|false|null]
  interrogation-ledger.sh report    [--ledger PATH] [--floor F] [--after N]
  interrogation-ledger.sh precision [--ledger PATH] [--floor F] [--after N]

Feature is INERT unless AUTOSPEC_REUSE_LENS=1.

record appends one JSONL line; write failure warns and exits 0 (never blocks).
report/precision compute per-trigger precision = upheld BLOCKs / total BLOCKs.
A trigger with precision < floor after >= N runs is flagged DEMOTE (auto-demote
to ADVISE).

Environment:
  AUTOSPEC_REUSE_LENS              set to "1" to arm (default: off)
  AUTOSPEC_REUSE_PRECISION_FLOOR   precision floor (default: 0.6)
  AUTOSPEC_REUSE_DEMOTE_AFTER      min runs before demotion (default: 10)
  AUTOSPEC_INTERROGATION_LEDGER    override default ledger path
EOF
}

die() {
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit 1
}

warn() {
    printf '%s: WARNING: %s\n' "$PROG" "$*" >&2
}

require_jq() {
    if ! command -v jq >/dev/null 2>&1; then
        die "jq is required but not found in PATH"
    fi
}

unix_now() {
    date +%s
}

# Resolve the default ledger path: .autospec/interrogation-ledger.jsonl under
# the git repo root, or under cwd if not in a git repo.
default_ledger_path() {
    local root=""
    root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
    printf '%s/.autospec/interrogation-ledger.jsonl' "$root"
}

# Return the effective ledger path: env override takes precedence.
effective_ledger_path() {
    if [ -n "${AUTOSPEC_INTERROGATION_LEDGER:-}" ]; then
        printf '%s' "$AUTOSPEC_INTERROGATION_LEDGER"
    else
        default_ledger_path
    fi
}

# ── subcommand: record ────────────────────────────────────────────────────────

cmd_record() {
    local issue="" pr="" trigger="" verdict="" upheld="null"
    while [ $# -gt 0 ]; do
        case "$1" in
            --issue)   issue="$2";   shift 2 ;;
            --pr)      pr="$2";      shift 2 ;;
            --trigger) trigger="$2"; shift 2 ;;
            --verdict) verdict="$2"; shift 2 ;;
            --upheld)  upheld="$2";  shift 2 ;;
            *) die "record: unknown option: $1" ;;
        esac
    done

    [ -n "$issue" ]   || die "record: --issue is required"
    [ -n "$pr" ]      || die "record: --pr is required"
    [ -n "$trigger" ] || die "record: --trigger is required"
    [ -n "$verdict" ] || die "record: --verdict is required"

    require_jq

    local upheld_val
    case "$upheld" in
        true|false) upheld_val="$upheld" ;;
        null|"")    upheld_val="null" ;;
        *)          die "record: --upheld must be true, false, or null (got: $upheld)" ;;
    esac

    local ledger
    ledger="$(effective_ledger_path)"

    local ts
    ts="$(unix_now)"

    # Build JSONL line. Use jq --arg for all string fields (injection-safe);
    # ts and upheld are argjson (numeric/boolean/null, not strings).
    # -c = compact (single-line) output, required for valid JSONL.
    local line
    line="$(jq -cn \
        --argjson ts     "$ts" \
        --arg     issue  "$issue" \
        --arg     pr     "$pr" \
        --arg     trigger "$trigger" \
        --arg     verdict "$verdict" \
        --argjson upheld "$upheld_val" \
        '{ts: $ts, issue: $issue, pr: $pr, trigger: $trigger, verdict: $verdict, upheld: $upheld}')"

    # Create directory; warn and return on failure (never block the PR).
    if ! mkdir -p "$(dirname "$ledger")" 2>/dev/null; then
        warn "cannot create ledger directory $(dirname "$ledger"); verdict not recorded"
        return 0
    fi

    # Append; warn on failure.
    if ! printf '%s\n' "$line" >> "$ledger" 2>/dev/null; then
        warn "ledger write failure at $ledger; verdict not recorded"
        return 0
    fi
}

# ── subcommand: report / precision internal ───────────────────────────────────

# _compute_report <ledger> <floor> <after> <emit_demote_exit>
# Prints the precision table. When emit_demote_exit=1 also prints
# DEMOTE:<trigger> sentinel lines (used by cmd_precision for exit-code logic).
_compute_report() {
    local ledger="$1"
    local floor="$2"
    local after="$3"
    local emit_demote="$4"

    require_jq

    if [ ! -f "$ledger" ]; then
        printf 'interrogation-ledger: no ledger at %s\n' "$ledger"
        return 0
    fi

    # Slurp all JSONL lines into an array, group BLOCK rows by trigger, compute
    # precision = upheld_count / total_block_count. Flag for demotion when
    # total >= after AND precision < floor.
    #
    # floor and after are passed via --arg (not interpolated into test() patterns
    # — injection-safe per feedback_jq_test_regex_metachar_injection).
    jq -rs \
        --arg floor "$floor" \
        --arg after "$after" \
        '[.[] | select(.verdict == "BLOCK")]
        | group_by(.trigger)
        | map({
            trigger:  .[0].trigger,
            total:    length,
            upheld:   ([.[] | select(.upheld == true)] | length)
          })
        | map(. + {
            precision: (if .total > 0 then (.upheld / .total) else 0 end)
          })
        | map(. + {
            demote: (
              .total >= ($after | tonumber) and
              .precision < ($floor | tonumber)
            )
          })
        | .[]
        | [
            .trigger,
            (.total    | tostring),
            (.upheld   | tostring),
            ((.precision * 100 | floor | tostring) + "%"),
            (if .demote then "DEMOTE" else "ok" end)
          ]
        | @tsv
        ' "$ledger" | while IFS=$'\t' read -r trig total upheld pct status; do
        printf '%-35s  blocks=%-4s  upheld=%-4s  precision=%-6s  %s\n' \
            "$trig" "$total" "$upheld" "$pct" "$status"
        if [ "$emit_demote" = "1" ] && [ "$status" = "DEMOTE" ]; then
            printf 'DEMOTE:%s\n' "$trig"
        fi
    done
}

cmd_report() {
    local ledger="" floor="" after=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --ledger) ledger="$2"; shift 2 ;;
            --floor)  floor="$2";  shift 2 ;;
            --after)  after="$2";  shift 2 ;;
            *) die "report: unknown option: $1" ;;
        esac
    done
    if [ -z "$ledger" ]; then ledger="$(effective_ledger_path)"; fi
    if [ -z "$floor" ];  then floor="$PRECISION_FLOOR"; fi
    if [ -z "$after" ];  then after="$DEMOTE_AFTER"; fi

    _compute_report "$ledger" "$floor" "$after" 0
}

cmd_precision() {
    local ledger="" floor="" after=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --ledger) ledger="$2"; shift 2 ;;
            --floor)  floor="$2";  shift 2 ;;
            --after)  after="$2";  shift 2 ;;
            *) die "precision: unknown option: $1" ;;
        esac
    done
    if [ -z "$ledger" ]; then ledger="$(effective_ledger_path)"; fi
    if [ -z "$floor" ];  then floor="$PRECISION_FLOOR"; fi
    if [ -z "$after" ];  then after="$DEMOTE_AFTER"; fi

    # Capture combined output (table rows + DEMOTE sentinels) into a temp file
    # so we can both print the table and check for DEMOTE markers.
    # Use a real temp file (macOS bash 3.2: no process substitution in [ -f ]).
    local tmp
    tmp="$(mktemp /tmp/interrogation-ledger-precision.XXXXXX)"
    _compute_report "$ledger" "$floor" "$after" 1 > "$tmp"

    # Print the report (without the sentinel lines).
    grep -v '^DEMOTE:' "$tmp" || true

    local has_demote=0
    if grep -q '^DEMOTE:' "$tmp" 2>/dev/null; then
        has_demote=1
    fi
    rm -f "$tmp"

    if [ "$has_demote" -eq 1 ]; then
        return 1
    fi
    return 0
}

# ── main ─────────────────────────────────────────────────────────────────────

main() {
    # Gate: when AUTOSPEC_REUSE_LENS is not "1", all subcommands are inert.
    # This ensures lint/reviewer/commit-loop behavior is byte-identical to today
    # when the flag is unset (proved by the flag-off tests in tests/reuse-lens/).
    if [ "${AUTOSPEC_REUSE_LENS:-}" != "1" ]; then
        exit 0
    fi

    if [ $# -eq 0 ]; then
        usage >&2
        exit 1
    fi

    local subcmd="$1"; shift
    case "$subcmd" in
        record)    cmd_record    "$@" ;;
        report)    cmd_report    "$@" ;;
        precision) cmd_precision "$@" ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown subcommand: $subcmd (expected: record | report | precision)" ;;
    esac
}

main "$@"

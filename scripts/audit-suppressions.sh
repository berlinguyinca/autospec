#!/usr/bin/env bash
# scripts/audit-suppressions.sh — policy layer over `cargo audit` (issue #4111).
#
# A bare `cargo audit` fails hard on every loaded advisory that has no fixed
# upgrade, and RUSTSEC-2023-0071 (Marvin Attack, rsa 0.9.10) has no upstream
# fix, so the CI audit job cannot run bare `cargo audit`. This script keeps
# the gate live for new advisories while recording the exceptions:
#
#   - One suppression file per advisory under config/audit-suppressions/
#     (override with --dir). Every entry MUST carry non-empty `advisory`,
#     `since`, `reason`, `removal_trigger`, and `dependency_path`. An entry
#     missing its reason or its removal trigger is rejected fail-closed
#     (exit 2) — an unexplained ignore is a policy error, not a pass.
#   - `gate` runs cargo-audit and exits non-zero unless every failing
#     advisory has a valid suppression. A cargo-audit failure that names no
#     RUSTSEC advisory (tool error, advisories not loaded) is a hard
#     failure, never a pass.
#   - `report` lists every active suppression with its age in days so a
#     temporary exception cannot quietly become permanent policy. The CI
#     audit job runs it on every run (.github/workflows/rust.yml).
#
# Usage:
#   scripts/audit-suppressions.sh gate     [--dir DIR] [--today YYYY-MM-DD]
#   scripts/audit-suppressions.sh report   [--dir DIR] [--today YYYY-MM-DD]
#   scripts/audit-suppressions.sh validate [--dir DIR] [--today YYYY-MM-DD]
#   scripts/audit-suppressions.sh --help
#
# Entry format (one `key: value` per line, file named <ADVISORY-ID>.txt):
#   advisory: RUSTSEC-2023-0071
#   since: 2026-09-10
#   reason: <stated reason, required>
#   removal_trigger: <condition under which this suppression is removed, required>
#   dependency_path: <path from the workspace crate to the vulnerable crate, required>
#
# A reachability-based reason must name the dependency path and the code
# that would need to change for the argument to stop holding; that part is
# enforced by review per docs/runbooks/audit-suppressions.md.
#
# Exit codes: 0 pass; 1 advisory failure (an unsuppressed advisory, or a
# cargo-audit failure naming no advisory); 2 invalid suppression entry;
# 3 usage error or no cargo-audit/cargo binary found.

set -eu

SCRIPT_PATH="$0"
# Pure-bash dirname so the gate works even under a restricted PATH (the
# no-cargo-audit test runs the script with an empty bin dir on PATH).
case "$SCRIPT_PATH" in
    */*) SCRIPT_DIR=$(cd "${SCRIPT_PATH%/*}" && pwd) ;;
    *) SCRIPT_DIR=$(pwd) ;;
esac
ROOT_DIR=$(cd "$SCRIPT_DIR/.." && pwd)

DEFAULT_DIR="$ROOT_DIR/config/audit-suppressions"

usage() {
    cat <<'EOF'
Usage: audit-suppressions.sh <gate|report|validate> [--dir DIR] [--today YYYY-MM-DD]

Subcommands:
  gate      Run cargo-audit; fail unless every failing advisory is suppressed
            by a valid entry in DIR (default: config/audit-suppressions).
  report    List every active suppression with its age in days; fail if any
            entry is invalid. Runs in CI on every audit-job execution.
  validate  Check every entry in DIR for required metadata; print findings.

Flags:
  --dir DIR       Suppression directory (default: config/audit-suppressions).
  --today DATE    Reference date YYYY-MM-DD for age and future-date checks
                  (default: date +%F).

Exit codes: 0 pass; 1 advisory failure; 2 invalid suppression entry;
3 usage error or missing cargo-audit/cargo.
EOF
}

die() { # die CODE MESSAGE
    printf 'audit-suppressions.sh: %s\n' "$2" >&2
    exit "$1"
}

trim() {
    local s="$1"
    s="${s#"${s%%[![:space:]]*}"}"
    s="${s%"${s##*[![:space:]]}"}"
    printf '%s' "$s"
}

is_leap() {
    local y="$1"
    { [ $((y % 4)) -eq 0 ] && [ $((y % 100)) -ne 0 ]; } || [ $((y % 400)) -eq 0 ]
}

days_in_month() { # days_in_month YEAR MONTH
    local y="$1" m="$2"
    case "$m" in
        1 | 3 | 5 | 7 | 8 | 10 | 12) printf '31' ;;
        4 | 6 | 9 | 11) printf '30' ;;
        2) if is_leap "$y"; then printf '29'; else printf '28'; fi ;;
        *) return 1 ;;
    esac
}

# Days since 1970-01-01 for a civil date (Hinnant days_from_civil), integer
# arithmetic only so the script runs on both GNU and BSD hosts.
days_from_civil() { # days_from_civil YEAR MONTH DAY
    local y="$1" m="$2" d="$3" era era_y m_y
    if [ "$m" -le 2 ]; then
        y=$((y - 1))
    fi
    if [ "$y" -ge 0 ]; then
        era=$((y / 400))
    else
        era=$(((y - 399) / 400))
    fi
    era_y=$((y - era * 400))
    m_y=$(((m + 9) % 12))
    printf '%s' $((era * 146097 + era_y * 365 + era_y / 4 - era_y / 100 + (m_y * 306 + 5) / 10 + d - 1 - 719468))
}

valid_calendar_date() { # valid_calendar_date YYYY-MM-DD
    local s="$1"
    [[ $s =~ ^([0-9]{4})-([0-9]{2})-([0-9]{2})$ ]] || return 1
    local y=$((10#${BASH_REMATCH[1]})) m=$((10#${BASH_REMATCH[2]})) d=$((10#${BASH_REMATCH[3]}))
    local dim
    dim=$(days_in_month "$y" "$m") || return 1
    [ "$d" -ge 1 ] && [ "$d" -le "$dim" ]
}

# ── Entry parsing / validation ────────────────────────────────────────────────
# parse_entry FILE — populate E_ADVISORY E_SINCE E_REASON E_REMOVAL_TRIGGER
# E_DEPENDENCY_PATH from the first `key: value` line of each key.
parse_entry() {
    local file="$1"
    E_ADVISORY=""
    E_SINCE=""
    E_REASON=""
    E_REMOVAL_TRIGGER=""
    E_DEPENDENCY_PATH=""
    local line key val
    while IFS= read -r line || [ -n "$line" ]; do
        if [[ $line =~ ^([a-z_]+):[[:space:]]*(.*)$ ]]; then
            key="${BASH_REMATCH[1]}"
            val="$(trim "${BASH_REMATCH[2]}")"
            case "$key" in
                advisory) [ -n "$E_ADVISORY" ] || E_ADVISORY="$val" ;;
                since) [ -n "$E_SINCE" ] || E_SINCE="$val" ;;
                reason) [ -n "$E_REASON" ] || E_REASON="$val" ;;
                removal_trigger) [ -n "$E_REMOVAL_TRIGGER" ] || E_REMOVAL_TRIGGER="$val" ;;
                dependency_path) [ -n "$E_DEPENDENCY_PATH" ] || E_DEPENDENCY_PATH="$val" ;;
            esac
        fi
    done <"$file"
}

# check_entry FILE TODAY — print one finding line per problem; no output
# means the entry is complete. Never returns non-zero (callers count lines).
check_entry() {
    local file="$1" today="$2"
    local stem="${file##*/}"
    stem="${stem%.*}"
    parse_entry "$file"
    local findings=""
    if ! [[ $stem =~ ^RUSTSEC-[0-9]{4}-[0-9]{4}$ ]]; then
        findings="${findings}filename must be RUSTSEC-YYYY-NNNN.txt"
    fi
    if [ -z "$E_ADVISORY" ]; then
        findings="${findings}missing required key: advisory"
    elif [ "$E_ADVISORY" != "$stem" ]; then
        findings="${findings}advisory ($E_ADVISORY) does not match filename ($stem)"
    fi
    if [ -z "$E_SINCE" ]; then
        findings="${findings}missing required key: since"
    elif ! valid_calendar_date "$E_SINCE"; then
        findings="${findings}since ($E_SINCE) is not a valid YYYY-MM-DD calendar date"
    fi
    if [ -n "$E_SINCE" ] && valid_calendar_date "$E_SINCE" && ! valid_calendar_date "$today"; then
        findings="${findings}today ($today) is not a valid YYYY-MM-DD calendar date"
    fi
    if [ -n "$E_SINCE" ] && valid_calendar_date "$E_SINCE" && valid_calendar_date "$today"; then
        local sy=$((10#${E_SINCE:0:4})) sm=$((10#${E_SINCE:5:2})) sd=$((10#${E_SINCE:8:2}))
        local ty=$((10#${today:0:4})) tm=$((10#${today:5:2})) td=$((10#${today:8:2}))
        if [ "$(days_from_civil "$sy" "$sm" "$sd")" -gt "$(days_from_civil "$ty" "$tm" "$td")" ]; then
            findings="${findings}since ($E_SINCE) is after today ($today)"
        fi
    fi
    if [ -z "$E_REASON" ]; then
        findings="${findings}missing required key: reason (a suppression without a stated reason is rejected)"
    fi
    if [ -z "$E_REMOVAL_TRIGGER" ]; then
        findings="${findings}missing required key: removal_trigger (a suppression without a removal trigger is rejected)"
    fi
    if [ -z "$E_DEPENDENCY_PATH" ]; then
        findings="${findings}missing required key: dependency_path"
    fi
    if [ -n "$findings" ]; then
        printf 'AUDIT_SUPPRESSION_INVALID:%s: %s\n' "$file" "$findings"
    fi
}

# list_entry_files DIR — print regular, non-hidden files in DIR (sorted).
list_entry_files() {
    local dir="$1" f
    [ -d "$dir" ] || return 0
    for f in "$dir"/*; do
        [ -f "$f" ] && printf '%s\n' "$f"
    done
}

# validate_all DIR TODAY — print findings for every entry; returns 0 only
# if there are none.
validate_all() {
    local dir="$1" today="$2"
    local file rc=0
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        if [ -n "$(check_entry "$file" "$today")" ]; then
            rc=1
        fi
    done < <(list_entry_files "$dir")
    return "$rc"
}

entry_field() { # entry_field DIR ID KEY
    local dir="$1" id="$2" key="$3" f
    for f in "$dir/$id".*; do
        [ -f "$f" ] || continue
        parse_entry "$f"
        case "$key" in
            advisory) printf '%s' "$E_ADVISORY" ;;
            since) printf '%s' "$E_SINCE" ;;
            reason) printf '%s' "$E_REASON" ;;
            removal_trigger) printf '%s' "$E_REMOVAL_TRIGGER" ;;
            dependency_path) printf '%s' "$E_DEPENDENCY_PATH" ;;
        esac
        return 0
    done
    return 1
}

has_entry() { # has_entry DIR ID
    local dir="$1" id="$2" f
    for f in "$dir/$id".*; do
        [ -f "$f" ] && return 0
    done
    return 1
}

count_entries() {
    local dir="$1" n=0 f
    while IFS= read -r f; do
        [ -n "$f" ] && n=$((n + 1))
    done < <(list_entry_files "$dir")
    printf '%s' "$n"
}

# ── Subcommands ───────────────────────────────────────────────────────────────

run_validate() { # run_validate DIR TODAY
    local dir="$1" today="$2"
    local file
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        check_entry "$file" "$today"
    done < <(list_entry_files "$dir")
    validate_all "$dir" "$today"
}

run_report() { # run_report DIR TODAY
    local dir="$1" today="$2"
    local file stem count=0 invalid=0
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        count=$((count + 1))
        stem="${file##*/}"
        local bad
        bad="$(check_entry "$file" "$today")"
        if [ -n "$bad" ]; then
            printf '%s\n' "$bad"
            invalid=1
            continue
        fi
        parse_entry "$file"
        local sy="${E_SINCE:0:4}" sm="${E_SINCE:5:2}" sd="${E_SINCE:8:2}"
        local ty="${today:0:4}" tm="${today:5:2}" td="${today:8:2}"
        local age=$(( $(days_from_civil "$((10#$ty))" "$((10#$tm))" "$((10#$td))") - $(days_from_civil "$((10#$sy))" "$((10#$sm))" "$((10#$sd))") ))
        printf '== %s == (age %s days, since %s)\n' "$E_ADVISORY" "$age" "$E_SINCE"
        printf '  dependency_path: %s\n' "$E_DEPENDENCY_PATH"
        printf '  reason: %s\n' "$E_REASON"
        printf '  removal_trigger: %s\n' "$E_REMOVAL_TRIGGER"
    done < <(list_entry_files "$dir")
    if [ "$count" -eq 0 ]; then
        printf 'no active suppressions (%s)\n' "$dir"
    fi
    return "$invalid"
}

run_gate() { # run_gate DIR TODAY
    local dir="$1" today="$2"
    local bad=0
    validate_all "$dir" "$today" || bad=1
    if [ "$bad" -ne 0 ]; then
        while IFS= read -r file; do
            [ -n "$file" ] || continue
            check_entry "$file" "$today"
        done < <(list_entry_files "$dir")
        die 2 "invalid suppression entries — fix the metadata before gating (reason and removal_trigger are mandatory)"
    fi

    local audit_cmd=()
    # Prefer `cargo audit` unconditionally when cargo exists. The standalone
    # `cargo-audit` binary takes its own subcommand name as argv[1] when
    # invoked DIRECTLY (cargo runs `cargo-audit audit ...`; bare `cargo-audit`
    # prints usage and exits 2), which is the class of bug #4164 fixed in a
    # branch that only CI ever selected -- the broken branch ran exactly where
    # nobody runs it interactively, and the correct one ran everywhere else.
    # Removing the environment-selected preference removes the class: `cargo
    # audit` is the only form on hosts with cargo, and the standalone binary
    # is a fallback for hosts without one.
    if command -v cargo >/dev/null 2>&1; then
        audit_cmd=(cargo audit)
    elif command -v cargo-audit >/dev/null 2>&1; then
        audit_cmd=(cargo-audit audit)
    else
        die 3 "neither cargo-audit nor cargo found in PATH — install cargo-audit (cargo install cargo-audit --locked)"
    fi

    local output rc=0
    output="$("${audit_cmd[@]}" 2>&1)" || rc=$?
    if [ "$rc" -eq 0 ]; then
        printf 'cargo-audit: no vulnerabilities found\n'
        local n
        n="$(count_entries "$dir")"
        if [ "$n" -gt 0 ]; then
            printf '%s suppression(s) active — see `bash scripts/audit-suppressions.sh report`\n' "$n"
        fi
        exit 0
    fi

    local ids
    ids="$(printf '%s\n' "$output" | grep -oE 'RUSTSEC-[0-9]{4}-[0-9]{4}' | LC_ALL=C sort -u)"
    if [ -z "$ids" ]; then
        printf 'cargo-audit exited %s without naming any RUSTSEC advisory — failing closed:\n' "$rc" >&2
        printf '%s\n' "$output" >&2
        exit 1
    fi

    local id unsuppressed=0
    while IFS= read -r id; do
        [ -n "$id" ] || continue
        if has_entry "$dir" "$id"; then
            printf 'AUDIT_SUPPRESSED:%s: reason: %s\n' "$id" "$(entry_field "$dir" "$id" reason)"
            printf 'AUDIT_SUPPRESSED:%s: removal_trigger: %s\n' "$id" "$(entry_field "$dir" "$id" removal_trigger)"
        else
            printf 'AUDIT_UNSUPPRESSED:%s: no valid suppression in %s — add one per docs/runbooks/audit-suppressions.md or fix the dependency\n' "$id" "$dir"
            unsuppressed=1
        fi
    done <<<"$ids"
    exit "$unsuppressed"
}

# ── Argument parsing ──────────────────────────────────────────────────────────

[ $# -ge 1 ] || { usage >&2; exit 3; }
CMD="$1"
shift

if [ "$CMD" = "-h" ] || [ "$CMD" = "--help" ]; then
    usage
    exit 0
fi

DIR_ARG=""
TODAY=""
while [ $# -gt 0 ]; do
    case "$1" in
        --dir)
            [ $# -ge 2 ] || die 3 "--dir requires a value"
            DIR_ARG="$2"
            shift 2
            ;;
        --today)
            [ $# -ge 2 ] || die 3 "--today requires a value"
            TODAY="$2"
            shift 2
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            die 3 "unknown argument: $1"
            ;;
    esac
done

[ -n "$DIR_ARG" ] || DIR_ARG="$DEFAULT_DIR"
[ -n "$TODAY" ] || TODAY="$(date +%F)"
valid_calendar_date "$TODAY" || die 3 "today ($TODAY) is not a valid YYYY-MM-DD calendar date"

case "$CMD" in
    gate) run_gate "$DIR_ARG" "$TODAY" ;;
    report)
        run_report "$DIR_ARG" "$TODAY" || die 2 "invalid suppression entries — fix the metadata before reporting (reason and removal_trigger are mandatory)"
        ;;
    validate)
        run_validate "$DIR_ARG" "$TODAY" || die 2 "invalid suppression entries — fix the metadata (reason and removal_trigger are mandatory)"
        ;;
    *)
        usage >&2
        die 3 "unknown subcommand: $CMD"
        ;;
esac

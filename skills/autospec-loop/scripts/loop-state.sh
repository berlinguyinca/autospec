#!/usr/bin/env bash
# loop-state.sh — atomic read/update of loop-state.json.
#
# Per docs/specs/2026-06-02-autospec-loop-design.md (§Phase 2):
# State file at ~/.autospec/loop/<repo-slug>/loop-state.json.
# Writes via mktemp + mv (atomic). Inline cleanup — no RETURN trap.
# One-sided conditionals use if/then/fi (no set -e short-circuit footgun).
#
# Schema 1:
#   { "schema": 1, "goal": str, "check": cmd|null, "mode": "cumulative"|"polling",
#     "max_iters": 25, "no_progress_K": 3, "iter": 0, "no_progress_count": 0,
#     "progress": [], "last_result": "", "done": false, "exit_reason": null }
#
# Subcommands:
#   init   --repo <o/n> --goal <str> --check <cmd|null> [--mode cumulative|polling]
#                       [--max-iters N] [--no-progress-k N]
#   read   --repo <o/n>
#   update --repo <o/n> --key <field> --value <val>
#   path   --repo <o/n>
#
# Environment:
#   AUTOSPEC_LOOP_STATE_DIR   base dir (default: ~/.autospec/loop)

set -eu

STATE_BASE="${AUTOSPEC_LOOP_STATE_DIR:-$HOME/.autospec/loop}"

err()  { printf 'loop-state: %s\n' "$1" >&2; }
die()  { err "$1"; exit 1; }

usage() {
    cat <<'EOF'
Usage:
  loop-state.sh init   --repo <owner/name> --goal <str> --check <cmd|null>
                       [--mode cumulative|polling] [--max-iters N] [--no-progress-k N]
  loop-state.sh read   --repo <owner/name>
  loop-state.sh update --repo <owner/name> --key <field> --value <val>
  loop-state.sh path   --repo <owner/name>
EOF
}

# Source the canonical repo-slug helper (F4). Reader and writer share this one
# repo_slug(), so routing it through canonical_slug migrates both atomically.
_LS_SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
for _rs_cand in \
    "${AUTOSPEC_REPO_SLUG_SH:-}" \
    "${_LS_SELF_DIR}/repo-slug.sh" \
    "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-slug.sh" \
    "${_LS_SELF_DIR}/../../../scripts/repo-slug.sh"; do
    if [ -n "$_rs_cand" ] && [ -f "$_rs_cand" ]; then
        # shellcheck source=/dev/null
        . "$_rs_cand"
        break
    fi
done

repo_slug() {
    repo="${1:-}"
    [ -n "$repo" ] || die "repo is required"
    case "$repo" in
        */*)
            if command -v canonical_slug >/dev/null 2>&1; then
                canonical_slug "$repo"
            else
                printf '%s' "$repo" | sed 's#/#__#'
            fi
            ;;
        *) printf '%s' "$repo" ;;   # slashless input has no canonical form
    esac
}

state_dir() {
    slug="$(repo_slug "$1")"
    printf '%s/%s' "$STATE_BASE" "$slug"
}

state_path() {
    printf '%s/loop-state.json' "$(state_dir "$1")"
}

now_iso() { date -u +'%Y-%m-%dT%H:%M:%SZ'; }

# write_state REPO JSON_STRING — atomic tmp+mv write, inline cleanup.
write_state() {
    repo="$1"
    json="$2"
    dir="$(state_dir "$repo")"
    mkdir -p "$dir"
    dest="$(state_path "$repo")"
    tmp="$(mktemp "${dest}.XXXXXX")"
    if ! printf '%s\n' "$json" > "$tmp"; then
        rm -f "$tmp"
        die "failed to write state to tmp file"
    fi
    if ! mv "$tmp" "$dest"; then
        rm -f "$tmp"
        die "failed to atomically rename tmp to state file"
    fi
}

# ── subcommand: init ──────────────────────────────────────────────────────────

cmd_init() {
    repo=""
    goal=""
    check_val=""
    mode="cumulative"
    max_iters=25
    no_progress_k=3

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo)          repo="${2:-}";          shift 2 ;;
            --goal)          goal="${2:-}";          shift 2 ;;
            --check)         check_val="${2:-}";     shift 2 ;;
            --mode)          mode="${2:-cumulative}"; shift 2 ;;
            --max-iters)     max_iters="${2:-25}";   shift 2 ;;
            --no-progress-k) no_progress_k="${2:-3}"; shift 2 ;;
            *) die "init: unknown option: $1" ;;
        esac
    done

    [ -n "$repo" ]     || die "init: --repo is required"
    [ -n "$goal" ]     || die "init: --goal is required"
    [ -n "$check_val" ] || die "init: --check is required (use 'null' for no check)"

    # Build check JSON value: bare "null" -> JSON null, else quoted string.
    if [ "$check_val" = "null" ]; then
        check_json="null"
    else
        check_json="$(printf '%s' "$check_val" | jq -Rs '.')"
    fi

    now="$(now_iso)"
    json="$(jq -n \
        --argjson schema 1 \
        --arg     goal   "$goal" \
        --argjson check  "$check_json" \
        --arg     mode   "$mode" \
        --argjson max_iters       "$max_iters" \
        --argjson no_progress_K   "$no_progress_k" \
        --argjson iter            0 \
        --argjson no_progress_count 0 \
        --arg     last_result     "" \
        --argjson done            false \
        --arg     created_at      "$now" \
        --arg     updated_at      "$now" \
        '{
            schema: $schema,
            goal: $goal,
            check: $check,
            mode: $mode,
            max_iters: $max_iters,
            no_progress_K: $no_progress_K,
            iter: $iter,
            no_progress_count: $no_progress_count,
            progress: [],
            last_result: $last_result,
            done: $done,
            exit_reason: null,
            created_at: $created_at,
            updated_at: $updated_at
        }')"

    write_state "$repo" "$json"
}

# ── subcommand: read ──────────────────────────────────────────────────────────

cmd_read() {
    repo=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) repo="${2:-}"; shift 2 ;;
            *) die "read: unknown option: $1" ;;
        esac
    done
    [ -n "$repo" ] || die "read: --repo is required"
    path="$(state_path "$repo")"
    if [ ! -f "$path" ]; then
        err "read: state file not found: $path"
        exit 1
    fi
    cat "$path"
}

# ── subcommand: update ────────────────────────────────────────────────────────

cmd_update() {
    repo=""
    key=""
    value=""

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo)  repo="${2:-}";  shift 2 ;;
            --key)   key="${2:-}";   shift 2 ;;
            --value) value="${2:-}"; shift 2 ;;
            *) die "update: unknown option: $1" ;;
        esac
    done

    [ -n "$repo"  ] || die "update: --repo is required"
    [ -n "$key"   ] || die "update: --key is required"
    # value may be empty string — do not gate on it

    path="$(state_path "$repo")"
    if [ ! -f "$path" ]; then
        die "update: state file not found: $path"
    fi

    now="$(now_iso)"

    # Determine the correct jq type for the value.
    # Booleans and numbers are passed as raw JSON; strings as --arg.
    case "$value" in
        true|false)
            # JSON boolean
            updated="$(jq --arg key "$key" --argjson val "$value" \
                --arg now "$now" \
                '.[$key] = $val | .updated_at = $now' "$path")"
            ;;
        ''|*[!0-9]*)
            # String (including empty string and non-numeric)
            updated="$(jq --arg key "$key" --arg val "$value" \
                --arg now "$now" \
                '.[$key] = $val | .updated_at = $now' "$path")"
            ;;
        *)
            # Numeric
            updated="$(jq --arg key "$key" --argjson val "$value" \
                --arg now "$now" \
                '.[$key] = $val | .updated_at = $now' "$path")"
            ;;
    esac

    write_state "$repo" "$updated"
}

# ── subcommand: path ──────────────────────────────────────────────────────────

cmd_path() {
    repo=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) repo="${2:-}"; shift 2 ;;
            *) die "path: unknown option: $1" ;;
        esac
    done
    [ -n "$repo" ] || die "path: --repo is required"
    state_path "$repo"
}

# ── dispatch ──────────────────────────────────────────────────────────────────

subcommand="${1:-}"
if [ -z "$subcommand" ]; then
    usage >&2
    exit 1
fi
shift

case "$subcommand" in
    init)   cmd_init   "$@" ;;
    read)   cmd_read   "$@" ;;
    update) cmd_update "$@" ;;
    path)   cmd_path   "$@" ;;
    --help|-h) usage ;;
    *) usage >&2; exit 1 ;;
esac

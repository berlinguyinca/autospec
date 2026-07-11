#!/usr/bin/env bash
# autospec-run-registry.sh — durable registry of active autospec runs.
#
# Closes root-cause #3 of docs/specs/2026-06-03-crash-resume-design.md: the
# relaunch command (`AUTOSPEC_RESUME_COMMAND`, computed at
# skills/autospec-run/SKILL.md:244) only survives in a session env var, so a
# bare host/session crash loses it. /autospec-run writes this registry at
# monitor launch so the command is DURABLE across reboot. The supervisor (child
# 2) and `/autospec-resume` read it on a fresh start.
#
# Registry file (path-scoped by repo-slug, mirroring the heartbeat collision
# lesson — ~/.autospec/process-heartbeats collides across repos otherwise):
#   ~/.autospec/active-runs/<repo-slug>.json
#   {
#     "schema": 1,
#     "repo": "owner/name",
#     "repo_dir": "/abs/path/to/checkout",
#     "harness": "claude|codex|opencode",
#     "resume_command": "<exact non-interactive relaunch command>",
#     "host": "<hostname that registered this run>",
#     "registered_at": "<iso8601>",
#     "updated_at": "<iso8601>"
#   }
#
# Subcommands:
#   write  --repo <o/n> --repo-dir <dir> --harness <h> --command <cmd> [--host <h>]
#   read   --repo <o/n>          # print the registry JSON (empty if absent)
#   list                         # print every registry file path, one per line
#   prune  [--repo <o/n>]        # drop entries older than 24h
#   path   --repo <o/n>          # print the resolved registry file path
#
# Writes are two-line atomic (build temp, then mv). Entries older than 24h are
# treated as stale by `read`/`list` consumers; `prune` removes them.
#
# Environment:
#   AUTOSPEC_ACTIVE_RUNS_DIR   base dir (default: ~/.autospec/active-runs)

set -eu

REGISTRY_BASE="${AUTOSPEC_ACTIVE_RUNS_DIR:-$HOME/.autospec/active-runs}"
STALE_SECS="${AUTOSPEC_REGISTRY_STALE_SECS:-86400}"   # 24h

err() { printf 'autospec-run-registry: %s\n' "$1" >&2; }
die() { err "$1"; exit 1; }

usage() {
    cat <<'EOF'
Usage:
  autospec-run-registry.sh write --repo <owner/name> --repo-dir <dir> --harness <h> --command <cmd> [--host <h>]
  autospec-run-registry.sh read  --repo <owner/name>
  autospec-run-registry.sh list
  autospec-run-registry.sh prune [--repo <owner/name>]
  autospec-run-registry.sh path  --repo <owner/name>
EOF
}

# Canonical repo-slug helper (F4). This store's reader and writer share one
# repo_slug(), so routing it through canonical_slug migrates both atomically.
_REG_SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
for _rs_cand in \
    "${AUTOSPEC_REPO_SLUG_SH:-}" \
    "${_REG_SELF_DIR}/repo-slug.sh" \
    "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-slug.sh" \
    "${_REG_SELF_DIR}/../scripts/repo-slug.sh"; do
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

registry_path() {
    slug="$(repo_slug "$1")"
    printf '%s/%s.json' "$REGISTRY_BASE" "$slug"
}

now_iso() { date -u +'%Y-%m-%dT%H:%M:%SZ'; }

iso_to_epoch() {
    ts="$1"
    [ -n "$ts" ] || { echo 0; return; }
    date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$ts" +%s 2>/dev/null \
        || date -u -d "$ts" +%s 2>/dev/null \
        || echo 0
}

# entry_is_stale FILE -> exit 0 (true) when file's updated_at is older than
# STALE_SECS. A missing/unparseable updated_at counts as stale.
entry_is_stale() {
    file="$1"
    [ -f "$file" ] || return 0
    updated="$(jq -r '.updated_at // empty' "$file" 2>/dev/null || true)"
    epoch="$(iso_to_epoch "$updated")"
    [ "$epoch" -gt 0 ] || return 0
    now="$(date -u +%s)"
    age=$((now - epoch))
    [ "$age" -ge "$STALE_SECS" ]
}

cmd_write() {
    repo="" repo_dir="" harness="" command="" host=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo)     repo="${2:-}";     shift 2 ;;
            --repo-dir) repo_dir="${2:-}"; shift 2 ;;
            --harness)  harness="${2:-}";  shift 2 ;;
            --command)  command="${2:-}";  shift 2 ;;
            --host)     host="${2:-}";     shift 2 ;;
            *) die "write: unknown option: $1" ;;
        esac
    done
    [ -n "$repo" ]     || die "write: --repo is required"
    [ -n "$repo_dir" ] || die "write: --repo-dir is required"
    [ -n "$harness" ]  || die "write: --harness is required"
    [ -n "$command" ]  || die "write: --command is required"
    [ -n "$host" ]     || host="${AUTOSPEC_HOST:-$(hostname 2>/dev/null || echo "")}"

    path="$(registry_path "$repo")"
    mkdir -p "$REGISTRY_BASE"
    now="$(now_iso)"
    registered_at="$now"
    if [ -f "$path" ]; then
        prev="$(jq -r '.registered_at // empty' "$path" 2>/dev/null || true)"
        [ -n "$prev" ] && registered_at="$prev"
    fi

    tmp="$(mktemp "${path}.XXXXXX")"
    if ! jq -n \
        --arg repo "$repo" \
        --arg repo_dir "$repo_dir" \
        --arg harness "$harness" \
        --arg resume_command "$command" \
        --arg host "$host" \
        --arg registered_at "$registered_at" \
        --arg updated_at "$now" \
        '{schema:1, repo:$repo, repo_dir:$repo_dir, harness:$harness, resume_command:$resume_command, host:$host, registered_at:$registered_at, updated_at:$updated_at}' \
        > "$tmp"; then
        rm -f "$tmp"
        die "write: failed to build registry JSON"
    fi
    mv "$tmp" "$path"

    # Telemetry (issue #1772): fire-and-forget heartbeat emit after the
    # registry write. Guarded source — an absent shim/binary/DSN is a silent
    # no-op and never alters this command's exit status or output.
    _H="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"
    if [ -f "$_H/emit-event.sh" ]; then
        # shellcheck source=/dev/null
        . "$_H/emit-event.sh"
        emit_event heartbeat repo="$repo" issue="" step="" pr="" || true
    fi

    printf '%s\n' "$path"
}

cmd_read() {
    repo=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) repo="${2:-}"; shift 2 ;;
            *) die "read: unknown option: $1" ;;
        esac
    done
    [ -n "$repo" ] || die "read: --repo is required"
    path="$(registry_path "$repo")"
    [ -f "$path" ] || exit 0
    if entry_is_stale "$path"; then
        # Stale entries are not surfaced to consumers.
        exit 0
    fi
    cat "$path"
}

cmd_list() {
    [ -d "$REGISTRY_BASE" ] || exit 0
    for f in "$REGISTRY_BASE"/*.json; do
        [ -f "$f" ] || continue
        entry_is_stale "$f" && continue
        printf '%s\n' "$f"
    done
}

cmd_prune() {
    repo=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) repo="${2:-}"; shift 2 ;;
            *) die "prune: unknown option: $1" ;;
        esac
    done
    if [ -n "$repo" ]; then
        path="$(registry_path "$repo")"
        if [ -f "$path" ] && entry_is_stale "$path"; then
            rm -f "$path"
        fi
        exit 0
    fi
    [ -d "$REGISTRY_BASE" ] || exit 0
    for f in "$REGISTRY_BASE"/*.json; do
        [ -f "$f" ] || continue
        entry_is_stale "$f" && rm -f "$f"
    done
}

cmd_path() {
    repo=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) repo="${2:-}"; shift 2 ;;
            *) die "path: unknown option: $1" ;;
        esac
    done
    [ -n "$repo" ] || die "path: --repo is required"
    registry_path "$repo"
}

subcommand="${1:-}"
[ -n "$subcommand" ] || { usage >&2; exit 1; }
shift || true

case "$subcommand" in
    write) cmd_write "$@" ;;
    read)  cmd_read "$@" ;;
    list)  cmd_list "$@" ;;
    prune) cmd_prune "$@" ;;
    path)  cmd_path "$@" ;;
    --help|-h) usage ;;
    *) usage >&2; exit 1 ;;
esac

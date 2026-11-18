#!/usr/bin/env bash
# scripts/disk-hygiene.sh — disk hygiene for autospec worktrees, build output
# and Docker test containers. ON by default; every piece has an env opt-out.
#
# Subcommands:
#   cleanup-worktree [--force] [--reason R] <path>
#       Remove a linked worktree (and the target/ dir inside it) when the tree
#       is clean AND every commit is pushed. Dirty / unpushed work is KEPT and
#       logged. --force skips both checks (throwaway worktrees only).
#       Prefers `herdr-eng cleanup --scope worktree --path <p>` when installed
#       and supported; falls back to git worktree remove (+ rm of leftovers).
#       exit 0 removed / already gone / disabled, 1 kept, 2 refused or failed.
#   env [<repo-or-worktree>]
#       Print `export` lines for a Rust repo: a shared per-repo
#       CARGO_TARGET_DIR and CARGO_INCREMENTAL=0. Silent for non-Rust repos.
#   target-dir [<repo-or-worktree>]
#       Print the shared target dir (empty when not applicable).
#   preflight [--path P]... [--label L]
#       Free-space gate for builds. Below threshold: run herdr-eng
#       cleanup/build-cleanup if available, re-check, else exit 75 (park).
#   docker-args
#       Print the flags every test container must carry.
#
# Environment (all optional):
#   AUTOSPEC_DISK_HYGIENE=off        master switch, disables everything here
#   AUTOSPEC_DISK_CLEANUP=off        no automatic worktree cleanup
#   AUTOSPEC_SHARED_TARGET=off       no shared CARGO_TARGET_DIR
#   AUTOSPEC_DISK_PREFLIGHT=off      no free-space gate
#   AUTOSPEC_CARGO_TARGET_ROOT       shared target root (default
#                                    $HOME/.cache/autospec/targets)
#   AUTOSPEC_DISK_MIN_FREE_PCT       default 10   (low if free % is below)
#   AUTOSPEC_DISK_MIN_FREE_GIB       default 100  (or free GiB is below)
#   AUTOSPEC_HERDR_ENG               herdr-eng binary (default `herdr-eng`)
#
# Sourceable: `. disk-hygiene.sh` defines dh_* functions and runs nothing.
# Conventions: bash 3.2 safe, no RETURN traps.

DH_PROG="disk-hygiene.sh"

dh_log() { printf '%s\n' "$*" >&2; }

dh_off() { # $1 = specific switch value
    [ "${AUTOSPEC_DISK_HYGIENE:-on}" = "off" ] || [ "${1:-on}" = "off" ]
}

dh_herdr() { printf '%s' "${AUTOSPEC_HERDR_ENG:-herdr-eng}"; }

# True when herdr-eng exists AND understands its `cleanup` subcommand.
dh_herdr_has_cleanup() {
    local bin
    bin="$(dh_herdr)"
    command -v "$bin" >/dev/null 2>&1 || return 1
    "$bin" cleanup --help >/dev/null 2>&1
}

dh_repo_root() { # main (primary) checkout root for any path inside a repo
    local p="${1:-.}" common
    common="$(git -C "$p" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" || return 1
    case "$common" in
        */.git) printf '%s\n' "${common%/.git}" ;;
        *) printf '%s\n' "$common" ;;
    esac
}

dh_repo_slug() {
    local root name sum
    root="$(dh_repo_root "${1:-.}")" || return 1
    name="$(basename "$root" | tr -c 'A-Za-z0-9._-\n' '-')"
    sum="$(printf '%s' "$root" | cksum | cut -d' ' -f1)"
    printf '%s-%s\n' "$name" "$sum"
}

dh_is_rust() {
    local p="${1:-.}" top
    top="$(git -C "$p" rev-parse --show-toplevel 2>/dev/null)" || return 1
    [ -f "$top/Cargo.toml" ]
}

dh_herdr_manages_targets() {
    local bin
    bin="$(dh_herdr)"
    command -v "$bin" >/dev/null 2>&1 || return 1
    "$bin" cargo-host status 2>/dev/null | grep -Eq '"manage_targets": *true'
}

# Shared target dir, or empty when it must not be set.
dh_target_dir() {
    local p="${1:-.}" root slug
    dh_off "${AUTOSPEC_SHARED_TARGET:-on}" && return 0
    [ -z "${CARGO_TARGET_DIR:-}" ] || return 0
    dh_is_rust "$p" || return 0
    dh_herdr_manages_targets && return 0
    slug="$(dh_repo_slug "$p")" || return 0
    root="${AUTOSPEC_CARGO_TARGET_ROOT:-$HOME/.cache/autospec/targets}"
    printf '%s/%s\n' "$root" "$slug"
}

dh_env() {
    local p="${1:-.}" td
    dh_off && return 0
    td="$(dh_target_dir "$p")"
    if [ -n "$td" ]; then
        printf 'export CARGO_TARGET_DIR=%q\n' "$td"
    fi
    if dh_is_rust "$p" && [ -z "${CARGO_INCREMENTAL:-}" ]; then
        printf 'export CARGO_INCREMENTAL=0\n'
    fi
}

dh_docker_args() { printf '%s\n' '--rm --label herdr.ephemeral=1'; }

# Why a worktree must be kept: prints a reason, or nothing when safe to remove.
dh_keep_reason() {
    local wt="$1" porcelain unpushed
    porcelain="$(git -C "$wt" status --porcelain 2>/dev/null)" || { echo "status_failed"; return; }
    [ -z "$porcelain" ] || { echo "dirty"; return; }
    unpushed="$(git -C "$wt" log HEAD --not --remotes --oneline 2>/dev/null || echo x)"
    [ -z "$unpushed" ] || { echo "unpushed"; return; }
}

dh_cleanup_worktree() {
    local force=0 reason="exit" wt common primary keep bin
    while [ $# -gt 0 ]; do
        case "$1" in
            --force) force=1; shift ;;
            --reason) reason="$2"; shift 2 ;;
            *) break ;;
        esac
    done
    wt="${1:-}"
    [ -n "$wt" ] || { dh_log "$DH_PROG: cleanup-worktree: path required"; return 2; }
    dh_off "${AUTOSPEC_DISK_CLEANUP:-on}" && return 0
    [ -e "$wt" ] || return 0

    if ! git -C "$wt" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        dh_log "disk_hygiene:worktree_refused path=$wt why=not_a_git_worktree"
        return 2
    fi
    primary="$(dh_repo_root "$wt")" || return 2
    common="$(git -C "$wt" rev-parse --path-format=absolute --git-dir 2>/dev/null)"
    if [ "$common" = "$(git -C "$wt" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" ]; then
        dh_log "disk_hygiene:worktree_refused path=$wt why=primary_checkout"
        return 2
    fi

    if [ "$force" -eq 0 ]; then
        keep="$(dh_keep_reason "$wt")"
        if [ -n "$keep" ]; then
            dh_log "disk_hygiene:worktree_kept path=$wt why=$keep reason=$reason"
            return 1
        fi
    fi

    if dh_herdr_has_cleanup; then
        bin="$(dh_herdr)"
        "$bin" cleanup --scope worktree --path "$wt" >/dev/null 2>&1 || true
    fi
    if [ -e "$wt" ]; then
        git -C "$primary" worktree remove --force "$wt" >/dev/null 2>&1 || true
        # Leftovers (target/, untracked build output) the git removal skipped.
        [ -e "$wt" ] && rm -rf "$wt" 2>/dev/null
    fi
    git -C "$primary" worktree prune >/dev/null 2>&1 || true
    if [ -e "$wt" ]; then
        dh_log "disk_hygiene:worktree_remove_failed path=$wt"
        return 2
    fi
    dh_log "disk_hygiene:worktree_removed path=$wt reason=$reason"
    return 0
}

dh_free_line() { # $1 path -> "pct_free gib_free"
    local p="$1"
    while [ ! -e "$p" ] && [ "$p" != "/" ]; do p="$(dirname "$p")"; done
    df -Pk "$p" 2>/dev/null | awk 'NR==2 { tot=$2; av=$4; if (tot>0) printf "%d %d\n", av*100/tot, av/1048576 }'
}

dh_low_paths() { # echoes "path pct gib" for each low path
    local p line pct gib min_pct min_gib
    min_pct="${AUTOSPEC_DISK_MIN_FREE_PCT:-10}"
    min_gib="${AUTOSPEC_DISK_MIN_FREE_GIB:-100}"
    for p in "$@"; do
        line="$(dh_free_line "$p")"
        [ -n "$line" ] || continue
        pct="${line% *}"; gib="${line#* }"
        if [ "$pct" -lt "$min_pct" ] || [ "$gib" -lt "$min_gib" ]; then
            printf '%s %s %s\n' "$p" "$pct" "$gib"
        fi
    done
}

dh_preflight() {
    local label="build" paths=() low bin
    while [ $# -gt 0 ]; do
        case "$1" in
            --path) paths+=("$2"); shift 2 ;;
            --label) label="$2"; shift 2 ;;
            *) shift ;;
        esac
    done
    dh_off "${AUTOSPEC_DISK_PREFLIGHT:-on}" && return 0
    if [ "${#paths[@]}" -eq 0 ]; then
        paths=("$PWD" "${AUTOSPEC_CARGO_TARGET_ROOT:-$HOME/.cache/autospec/targets}")
    fi
    low="$(dh_low_paths "${paths[@]}")"
    if [ -n "$low" ]; then
        bin="$(dh_herdr)"
        if command -v "$bin" >/dev/null 2>&1; then
            if dh_herdr_has_cleanup; then
                for p in "${paths[@]}"; do
                    "$bin" cleanup --scope tmp --path "$p" >/dev/null 2>&1 || true
                done
            fi
            "$bin" build-cleanup run >/dev/null 2>&1 || true
        fi
        low="$(dh_low_paths "${paths[@]}")"
    fi
    if [ -n "$low" ]; then
        dh_log "disk_hygiene:disk_low_parked label=$label low=\"$(printf '%s' "$low" | tr '\n' ';')\" min_pct=${AUTOSPEC_DISK_MIN_FREE_PCT:-10} min_gib=${AUTOSPEC_DISK_MIN_FREE_GIB:-100}"
        return 75
    fi
    dh_log "disk_hygiene:preflight_ok label=$label"
    return 0
}

dh_main() {
    local cmd="${1:-}"
    [ $# -gt 0 ] && shift
    case "$cmd" in
        cleanup-worktree) dh_cleanup_worktree "$@" ;;
        env) dh_env "$@" ;;
        target-dir) dh_target_dir "$@" ;;
        preflight) dh_preflight "$@" ;;
        docker-args) dh_docker_args ;;
        ""|-h|--help) sed -n '2,40p' "$0" ;;
        *) dh_log "$DH_PROG: unknown subcommand: $cmd"; return 2 ;;
    esac
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    dh_main "$@"
    exit $?
fi

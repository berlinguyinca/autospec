#!/usr/bin/env bash
# scripts/disk-hygiene.sh — disk hygiene for autospec worktrees, build output
# and Docker test containers. ON by default; every piece has an env opt-out.
#
# Subcommands:
#   cleanup-worktree [--force] [--reason R] <path>
#       Remove a linked worktree (and the target/ dir inside it) when the tree
#       is clean AND every commit is pushed. Dirty / unpushed work is KEPT and
#       logged. --force skips both checks (throwaway worktrees only).
#       --checked: the caller already ran keep-reason and it was empty; skips
#       the network re-check but still re-verifies dirtiness locally.
#       --expect-head <oid>: keep the worktree (exit 1, why=head_changed) when
#       HEAD is no longer <oid>, i.e. the caller's judgement is stale.
#       The path must be exactly a registered, unlocked linked-worktree root
#       (never a subdirectory, the primary checkout, or a locked worktree).
#       Prefers `herdr-eng cleanup --scope worktree --path <p>` when installed
#       and supported; falls back to git worktree remove. Leftover untracked
#       files are removed only AFTER git removal succeeded, never as a fallback.
#       exit 0 removed / already gone / disabled, 1 kept, 2 refused or failed.
#   keep-reason <path>
#       Print why a worktree must be kept (dirty, unpushed, status_failed);
#       print nothing when it is safe to remove. Read-only apart from a
#       best-effort `git fetch --prune`.
#   env [<repo-or-worktree>]
#       Print `export CARGO_INCREMENTAL=0` for a Rust repo. CARGO_TARGET_DIR is
#       left alone (builds use the in-worktree target/, removed with it).
#   preflight [--path P]... [--label L]
#       Free-space gate for builds, over every given path plus $HOME and
#       $CARGO_TARGET_DIR. Below threshold: run herdr-eng cleanup/build-cleanup
#       if available, re-check, else exit 75 (park).
#   docker-args
#       Print the flags every test container must carry.
#
# Environment (all optional):
#   AUTOSPEC_DISK_HYGIENE=off        master switch, disables everything here
#   AUTOSPEC_DISK_CLEANUP=off        no automatic worktree cleanup
#   AUTOSPEC_DISK_PREFLIGHT=off      no free-space gate
#   AUTOSPEC_DISK_MIN_FREE_PCT       default 10   (a path is low when free space
#   AUTOSPEC_DISK_MIN_FREE_GIB       default 20    is below min(GIB, PCT% of the
#                                    volume size): small volumes park before they
#                                    are nearly full, big ones are not over-parked)
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

dh_is_rust() {
    local p="${1:-.}" top
    top="$(git -C "$p" rev-parse --show-toplevel 2>/dev/null)" || return 1
    [ -f "$top/Cargo.toml" ]
}

dh_env() {
    local p="${1:-.}"
    dh_off && return 0
    if dh_is_rust "$p" && [ -z "${CARGO_INCREMENTAL:-}" ]; then
        printf 'export CARGO_INCREMENTAL=0\n'
    fi
}

dh_docker_args() { printf '%s\n' '--rm --label herdr.ephemeral=1'; }

# Canonical absolute path (symlinks resolved) of an existing directory.
dh_abs() { (cd "$1" 2>/dev/null && pwd -P); }

# Registration state of <abs-path> in <primary>'s worktree list:
# prints "registered", "locked", or nothing when it is not a linked worktree.
dh_wt_state() {
    local primary="$1" want="$2" line path="" locked=0 first=1 found=""
    while IFS= read -r line; do
        case "$line" in
            "worktree "*)
                path="$(dh_abs "${line#worktree }" 2>/dev/null || true)"
                locked=0
                ;;
            locked*) locked=1 ;;
            "")
                if [ "$first" -eq 0 ] && [ -n "$path" ] && [ "$path" = "$want" ]; then
                    found=$((1 + locked)); break
                fi
                first=0; path=""
                ;;
        esac
    done < <(git -C "$primary" worktree list --porcelain 2>/dev/null; echo)
    case "$found" in
        1) echo registered ;;
        2) echo locked ;;
    esac
}

# Run a command with a timeout when one is available (timeout / gtimeout);
# without either it runs unbounded (BatchMode still stops ssh prompts).
dh_timeout() { # $1 = seconds, rest = command
    local secs="$1" t
    shift
    for t in timeout gtimeout; do
        if command -v "$t" >/dev/null 2>&1; then
            "$t" "$secs" "$@"
            return $?
        fi
    done
    "$@"
}

# Bounded, non-interactive `git fetch --prune origin` (best effort). Git's ssh
# precedence is GIT_SSH_COMMAND > core.sshCommand > GIT_SSH. A configured command
# keeps its identity and gets `-o BatchMode=yes -o ConnectTimeout=10` appended
# (unless GIT_SSH_VARIANT/ssh.variant says it is not OpenSSH-style); a GIT_SSH
# program and its variant are left untouched; with none of them set, plain ssh
# gets the same options.
dh_fetch() {
    local ssh_cmd="${GIT_SSH_COMMAND:-}" variant="${GIT_SSH_VARIANT:-}" opts="-o BatchMode=yes -o ConnectTimeout=10"
    local -a env_args=(GIT_TERMINAL_PROMPT=0)
    [ -n "$variant" ] || variant="$(git -C "$1" config --get ssh.variant 2>/dev/null || true)"
    [ -n "$ssh_cmd" ] || ssh_cmd="$(git -C "$1" config --get core.sshCommand 2>/dev/null || true)"
    if [ -n "$ssh_cmd" ]; then
        case "$variant" in
            ''|ssh|auto) env_args+=("GIT_SSH_COMMAND=$ssh_cmd $opts") ;;
            *) env_args+=("GIT_SSH_COMMAND=$ssh_cmd") ;;
        esac
    elif [ -z "${GIT_SSH:-}" ]; then
        env_args+=("GIT_SSH_COMMAND=ssh $opts")
    fi
    dh_timeout "${AUTOSPEC_DISK_FETCH_TIMEOUT_SECS:-30}" env "${env_args[@]}" \
        git -C "$1" fetch --prune -q origin >/dev/null 2>&1 || true
}

# Default branch ref that actually exists (origin/HEAD target, else origin/main|master).
dh_default_branch() {
    local wt="$1" d
    d="$(git -C "$wt" symbolic-ref -q --short refs/remotes/origin/HEAD 2>/dev/null || true)"
    if [ -n "$d" ] && git -C "$wt" rev-parse -q --verify "$d^{commit}" >/dev/null 2>&1; then
        printf '%s\n' "$d"
        return 0
    fi
    for d in origin/main origin/master; do
        if git -C "$wt" rev-parse -q --verify "$d^{commit}" >/dev/null 2>&1; then
            printf '%s\n' "$d"
            return 0
        fi
    done
    return 1
}

# Is HEAD's work safely on a remote? Fetches/prunes first (best effort) so
# stale remote-tracking refs do not make deleted/rewritten work look pushed.
# Fails closed: any doubt means "not pushed".
dh_pushed() {
    local wt="$1" head upstream branch default oid cherry merges
    dh_fetch "$wt"
    head="$(git -C "$wt" rev-parse HEAD 2>/dev/null)" || return 1
    branch="$(git -C "$wt" rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
    # The branch's own remote ref (same name); a `-b x origin/main` worktree's
    # configured upstream is the BASE branch and says nothing about pushing.
    upstream=""
    if [ -n "$branch" ] && [ "$branch" != HEAD ]; then
        upstream="$(git -C "$wt" for-each-ref --format='%(refname:short)' "refs/remotes/*/$branch" 2>/dev/null | head -1)"
    fi
    if [ -n "$upstream" ]; then
        git -C "$wt" merge-base --is-ancestor "$head" "$upstream" 2>/dev/null && return 0
    elif [ -z "$(git -C "$wt" log HEAD --not --remotes --oneline 2>/dev/null || echo x)" ]; then
        return 0
    fi
    # Squash/rebase-merged then deleted branch: PR MERGED at exactly this HEAD.
    if [ -n "$branch" ] && [ "$branch" != HEAD ] && command -v gh >/dev/null 2>&1; then
        oid="$(cd "$wt" && dh_timeout "${AUTOSPEC_DISK_GH_TIMEOUT_SECS:-30}" gh pr list --head "$branch" --state merged --limit 5 \
            --json headRefOid --jq '.[].headRefOid' 2>/dev/null || true)"
        printf '%s\n' "$oid" | grep -qx "$head" && return 0
    fi
    # Patch-equivalent to the default branch (cherry-pick / single-commit squash).
    # cherry never lists merge commits, so require there are none, and treat a
    # cherry failure (e.g. dangling origin/HEAD) as NOT pushed.
    default="$(dh_default_branch "$wt")" || return 1
    merges="$(git -C "$wt" rev-list --merges "$default..HEAD" 2>/dev/null)" || return 1
    [ -z "$merges" ] || return 1
    cherry="$(git -C "$wt" cherry "$default" HEAD 2>/dev/null)" || return 1
    printf '%s\n' "$cherry" | grep -q '^+' && return 1
    return 0
}

# Why a worktree must be kept: prints a reason, or nothing when safe to remove.
dh_dirty_reason() { # local-only check: status_failed / dirty / nothing
    local wt="$1" porcelain
    porcelain="$(git -C "$wt" status --porcelain 2>/dev/null)" || { echo "status_failed"; return; }
    [ -z "$porcelain" ] || echo "dirty"
}

dh_keep_reason() {
    local wt="$1" r
    r="$(dh_dirty_reason "$wt")"
    [ -z "$r" ] || { echo "$r"; return; }
    dh_pushed "$wt" || echo "unpushed"
}

dh_cleanup_worktree() {
    local force=0 checked=0 reason="exit" expect_head="" wt top primary state keep bin
    while [ $# -gt 0 ]; do
        case "$1" in
            --force) force=1; shift ;;
            --checked) checked=1; shift ;;
            --expect-head|--reason)
                # A guard option with no (or an empty) value fails closed: an
                # empty --expect-head must never mean "no HEAD check".
                if [ $# -lt 2 ] || [ -z "$2" ]; then
                    dh_log "$DH_PROG: cleanup-worktree: $1 requires a non-empty value"
                    return 2
                fi
                if [ "$1" = "--expect-head" ]; then expect_head="$2"; else reason="$2"; fi
                shift 2 ;;
            *) break ;;
        esac
    done
    wt="${1:-}"
    [ -n "$wt" ] || { dh_log "$DH_PROG: cleanup-worktree: path required"; return 2; }
    dh_off "${AUTOSPEC_DISK_CLEANUP:-on}" && return 0
    [ -e "$wt" ] || return 0

    # Canonicalise once: git (against the primary) and rm (against $PWD) must
    # agree on what is being removed.
    wt="$(dh_abs "$wt")" || { dh_log "disk_hygiene:worktree_refused path=$1 why=not_a_directory"; return 2; }
    if ! top="$(git -C "$wt" rev-parse --show-toplevel 2>/dev/null)"; then
        dh_log "disk_hygiene:worktree_refused path=$wt why=not_a_git_worktree"
        return 2
    fi
    if [ "$(dh_abs "$top")" != "$wt" ]; then
        dh_log "disk_hygiene:worktree_refused path=$wt why=not_worktree_root"
        return 2
    fi
    primary="$(dh_repo_root "$wt")" || return 2
    primary="$(dh_abs "$primary")"
    if [ "$wt" = "$primary" ] || [ "$(git -C "$wt" rev-parse --path-format=absolute --git-dir 2>/dev/null)" = "$(git -C "$wt" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" ]; then
        dh_log "disk_hygiene:worktree_refused path=$wt why=primary_checkout"
        return 2
    fi
    state="$(dh_wt_state "$primary" "$wt")"
    if [ -z "$state" ]; then
        dh_log "disk_hygiene:worktree_refused path=$wt why=not_a_registered_worktree"
        return 2
    fi
    if [ "$state" = locked ]; then
        dh_log "disk_hygiene:worktree_kept path=$wt why=locked reason=$reason"
        return 1
    fi

    # The caller judged this exact HEAD; a moved HEAD (adopted, new commit) is
    # unvalidated work, so keep it.
    if [ -n "$expect_head" ] && [ "$(git -C "$wt" rev-parse HEAD 2>/dev/null)" != "$expect_head" ]; then
        dh_log "disk_hygiene:worktree_kept path=$wt why=head_changed reason=$reason"
        return 1
    fi

    if [ "$force" -eq 0 ]; then
        if [ "$checked" -eq 1 ]; then
            keep="$(dh_dirty_reason "$wt")"
        else
            keep="$(dh_keep_reason "$wt")"
        fi
        if [ -n "$keep" ]; then
            dh_log "disk_hygiene:worktree_kept path=$wt why=$keep reason=$reason"
            return 1
        fi
    fi

    if dh_herdr_has_cleanup; then
        bin="$(dh_herdr)"
        "$bin" cleanup --scope worktree --path "$wt" >/dev/null 2>&1 || true
    fi
    if [ -n "$(dh_wt_state "$primary" "$wt")" ]; then
        if ! git -C "$primary" worktree remove --force "$wt" >/dev/null 2>&1; then
            dh_log "disk_hygiene:worktree_remove_failed path=$wt"
            return 2
        fi
    fi
    # Git removal succeeded (or herdr-eng did it); sweep untracked leftovers
    # (target/, build output) it skipped. Never reached after a failure.
    [ -e "$wt" ] && rm -rf "$wt" 2>/dev/null
    git -C "$primary" worktree prune >/dev/null 2>&1 || true
    if [ -e "$wt" ]; then
        dh_log "disk_hygiene:worktree_remove_failed path=$wt"
        return 2
    fi
    dh_log "disk_hygiene:worktree_removed path=$wt reason=$reason"
    return 0
}

dh_free_line() { # $1 path -> "avail_kib total_kib"
    local p="$1"
    while [ ! -e "$p" ] && [ "$p" != "/" ]; do p="$(dirname "$p")"; done
    df -Pk "$p" 2>/dev/null | awk 'NR==2 && $2>0 { printf "%d %d\n", $4, $2 }'
}

# A path is low when free space < min(MIN_FREE_GIB, MIN_FREE_PCT% of the volume).
dh_low_paths() { # echoes "path pct gib" for each low path
    local p line avail tot min_pct min_gib floor_kib pct_kib
    min_pct="${AUTOSPEC_DISK_MIN_FREE_PCT:-10}"
    min_gib="${AUTOSPEC_DISK_MIN_FREE_GIB:-20}"
    for p in "$@"; do
        line="$(dh_free_line "$p")"
        [ -n "$line" ] || continue
        avail="${line% *}"; tot="${line#* }"
        floor_kib=$((min_gib * 1048576))
        pct_kib=$((tot * min_pct / 100))
        [ "$pct_kib" -lt "$floor_kib" ] && floor_kib="$pct_kib"
        if [ "$avail" -lt "$floor_kib" ]; then
            printf '%s %s %s\n' "$p" $((avail * 100 / tot)) $((avail / 1048576))
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
    [ "${#paths[@]}" -gt 0 ] || paths=("$PWD")
    # Build output and caches live elsewhere than the worktree: check them too.
    [ -z "${CARGO_TARGET_DIR:-}" ] || paths+=("$CARGO_TARGET_DIR")
    [ -z "${HOME:-}" ] || paths+=("$HOME")
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
        dh_log "disk_hygiene:disk_low_parked label=$label low=\"$(printf '%s' "$low" | tr '\n' ';')\" min_pct=${AUTOSPEC_DISK_MIN_FREE_PCT:-10} min_gib=${AUTOSPEC_DISK_MIN_FREE_GIB:-20}"
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
        cleanup-enabled) ! dh_off "${AUTOSPEC_DISK_CLEANUP:-on}" ;;
        keep-reason) dh_keep_reason "$(dh_abs "${1:-.}")" ;;
        env) dh_env "$@" ;;
        preflight) dh_preflight "$@" ;;
        docker-args) dh_docker_args ;;
        ""|-h|--help) sed -n '2,50p' "$0" ;;
        *) dh_log "$DH_PROG: unknown subcommand: $cmd"; return 2 ;;
    esac
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    dh_main "$@"
    exit $?
fi

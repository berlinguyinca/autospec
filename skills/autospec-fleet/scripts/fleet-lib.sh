#!/usr/bin/env bash
# Shared helpers for autospec-fleet shell commands.

if [ -z "${BASH_VERSION:-}" ]; then
    printf 'fleet-lib.sh must be sourced by bash\n' >&2
    return 2 2>/dev/null || exit 2
fi

normalize_repo_url() {
    local input="${1:-}"
    local path=""
    local owner=""
    local repo=""

    case "$input" in
        https://github.com/*/*)
            path="${input#https://github.com/}"
            ;;
        git@github.com:*/*)
            path="${input#git@github.com:}"
            ;;
        *)
            printf 'fleet: unsupported repo URL: %s\n' "$input" >&2
            return 2
            ;;
    esac

    path="${path%.git}"
    if [[ "$path" =~ ^([^/]+)/([^/]+)$ ]]; then
        owner="${BASH_REMATCH[1]}"
        repo="${BASH_REMATCH[2]}"
    else
        printf 'fleet: unsupported repo URL: %s\n' "$input" >&2
        return 2
    fi

    if [ -z "$owner" ] || [ -z "$repo" ]; then
        printf 'fleet: unsupported repo URL: %s\n' "$input" >&2
        return 2
    fi

    printf '%s/%s\n' "$owner" "$repo"
}

repo_slug() {
    local normalized="${1:-}"

    case "$normalized" in
        */*) ;;
        *)
            printf 'fleet: repo slug requires owner/repo, got: %s\n' "$normalized" >&2
            return 2
            ;;
    esac

    printf '%s\n' "${normalized//\//__}"
}

repo_checkout_path() {
    local workspace="${1:-}"
    local normalized="${2:-}"
    local slug

    if [ -z "$workspace" ]; then
        printf 'fleet: workspace is required\n' >&2
        return 2
    fi

    slug="$(repo_slug "$normalized")" || return 2
    printf '%s/%s\n' "${workspace%/}" "$slug"
}

fleet_worker_id() {
    local node_id="${1:-}"
    local normalized="${2:-}"
    local slug

    [ -n "$node_id" ] || node_id="local"
    slug="$(repo_slug "$normalized")" || return 2
    printf 'fleet:%s:%s\n' "$node_id" "$slug"
}

autospec_run_command() {
    local profile="${1:-}"
    local worker_id="${2:-}"

    [ -n "$profile" ] || { printf 'fleet: profile is required\n' >&2; return 2; }
    [ -n "$worker_id" ] || { printf 'fleet: worker ID is required\n' >&2; return 2; }
    printf '/autospec-run --profile %s --worker-id %s\n' "$profile" "$worker_id"
}

# A fleet worker is a perpetual conductor, not a one-shot batch run: "ship
# this project" means keep draining the repo's queue until the board is
# done, not drain one batch and exit. This returns a *printable* command
# (shell-quoted via bash's printf %q) for --dry-run / log display only.
# fleet-run.sh's live spawn path invokes the binary directly by argv — it
# never eval's this string, because repo/checkout values come from an
# untrusted board/config.
fleet_worker_command() {
    local profile="${1:-}"
    local worker_id="${2:-}"
    local repo="${3:-}"
    local checkout="${4:-}"

    [ -n "$repo" ] || { printf 'fleet: repo is required\n' >&2; return 2; }
    [ -n "$checkout" ] || { printf 'fleet: checkout path is required\n' >&2; return 2; }
    # `start` already detaches by default (see usage: "Start the detached
    # autonomous conductor (default)."); the parser has no `--detach` case
    # at all, so passing it makes every spawn fail with "unknown argument:
    # --detach". Never add it back.
    printf 'autospec-autonomous start --repo-dir %s --repo %s\n' \
        "$(printf '%q' "$checkout")" "$(printf '%q' "$repo")"
}

# ── Fleet checkout provisioning ───────────────────────────────────────────────
# fleet-init.sh's non-dry-run path ensures a usable checkout exists at the
# deterministic path repo_checkout_path() computes. This is the ONLY path
# scheme provisioning uses — never invent a second one.
#
# Idempotence: a repo already checked out is fetched and fast-forwarded, not
# re-cloned. Safety: a dirty working tree, or an update that would not
# fast-forward, is skipped with a message — never reset/clean/force. A
# clone/fetch failure is per-repo (a code_health: marker to stderr) and never
# aborts the caller's loop over the rest of the fleet.

# True (0) when checkout_path is a path strictly inside workspace (a single
# path component appended to workspace, per repo_checkout_path's own
# construction) — never realpath, since the checkout may not exist yet.
# repo_slug() already guarantees the slug it hands back contains no "/", so
# this is defense-in-depth against a future change to that scheme, not the
# primary guarantee.
fleet_path_within_workspace() {
    local workspace="${1:-}"
    local checkout_path="${2:-}"
    local ws_norm="${workspace%/}"
    local rest

    case "$checkout_path" in
        "$ws_norm"/*) : ;;
        *) return 1 ;;
    esac

    rest="${checkout_path#"$ws_norm"/}"
    case "$rest" in
        */*|..|.|'') return 1 ;;
    esac
    return 0
}

# Clone repo_url into checkout_path (parent already created by caller).
# Prints a code_health: marker and returns non-zero on failure; never aborts.
fleet_clone_checkout() {
    local repo_url="${1:-}"
    local normalized="${2:-}"
    local checkout_path="${3:-}"

    if git clone --quiet -- "$repo_url" "$checkout_path" >/dev/null 2>&1; then
        printf 'fleet: cloned %s -> %s\n' "$normalized" "$checkout_path"
        return 0
    fi
    printf 'code_health:fleet_provision_clone_failed repo=%s path=%s\n' \
        "$normalized" "$checkout_path" >&2
    return 1
}

# Update an existing checkout in place: fetch, then fast-forward only.
# Never touches a checkout with local changes (staged, unstaged, or
# untracked), and never force-updates a history that would not fast-forward.
fleet_update_checkout() {
    local normalized="${1:-}"
    local checkout_path="${2:-}"
    local dirty

    dirty="$(git -C "$checkout_path" status --porcelain 2>/dev/null)" || {
        printf 'code_health:fleet_provision_status_failed repo=%s path=%s\n' \
            "$normalized" "$checkout_path" >&2
        return 1
    }
    if [ -n "$dirty" ]; then
        # This is a permanent, invisible stall in an unattended multi-week
        # fleet if it's only a plain `fleet:` line — the operator has no
        # signal to explain why a repo quietly stopped progressing. Loud
        # code_health: marker with a name distinct from the non-ff case
        # below, matching every sibling marker in this function
        # (fleet_provision_clone_failed, _fetch_failed, _status_failed,
        # _path_escape all carry the distinction in the NAME, not a shared
        # name plus a field) — dirty needs "go clean up the checkout";
        # non-ff needs "go resolve a diverged history"; an operator
        # grepping by marker name for one must not come up empty because
        # the other fired instead.
        printf 'code_health:fleet_provision_dirty_checkout repo=%s path=%s\n' \
            "$normalized" "$checkout_path" >&2
        printf 'fleet: %s: local changes present at %s; skipping update\n' \
            "$normalized" "$checkout_path"
        return 0
    fi

    if ! git -C "$checkout_path" fetch --quiet 2>/dev/null; then
        printf 'code_health:fleet_provision_fetch_failed repo=%s path=%s\n' \
            "$normalized" "$checkout_path" >&2
        return 1
    fi

    if git -C "$checkout_path" merge --ff-only --quiet FETCH_HEAD >/dev/null 2>&1; then
        printf 'fleet: %s: updated %s\n' "$normalized" "$checkout_path"
        return 0
    fi
    printf 'code_health:fleet_provision_not_fast_forward repo=%s path=%s\n' \
        "$normalized" "$checkout_path" >&2
    printf 'fleet: %s: update at %s would not fast-forward; skipping\n' \
        "$normalized" "$checkout_path"
    return 0
}

# Ensure a usable checkout exists for repo_url under workspace: clone if
# absent, fetch+ff-only-update if present. Per-repo failures are reported via
# code_health: markers and a non-zero return; callers must not let one
# repo's failure abort the loop over the rest of the fleet (if/then, never a
# one-sided `&&` on this call).
fleet_provision_repo() {
    local workspace="${1:-}"
    local repo_url="${2:-}"
    local normalized checkout_path parent

    normalized="$(normalize_repo_url "$repo_url")" || return 2
    checkout_path="$(repo_checkout_path "$workspace" "$normalized")" || return 2

    if ! fleet_path_within_workspace "$workspace" "$checkout_path"; then
        printf 'code_health:fleet_provision_path_escape repo=%s path=%s\n' \
            "$normalized" "$checkout_path" >&2
        return 1
    fi

    if [ -d "$checkout_path/.git" ]; then
        fleet_update_checkout "$normalized" "$checkout_path"
        return $?
    fi

    if [ -e "$checkout_path" ]; then
        printf 'code_health:fleet_provision_path_occupied repo=%s path=%s\n' \
            "$normalized" "$checkout_path" >&2
        return 1
    fi

    parent="$(dirname -- "$checkout_path")"
    if ! mkdir -p -- "$parent"; then
        printf 'code_health:fleet_provision_mkdir_failed repo=%s path=%s\n' \
            "$normalized" "$parent" >&2
        return 1
    fi
    fleet_clone_checkout "$repo_url" "$normalized" "$checkout_path"
}

# Resolve the autospec-autonomous binary: explicit override, then PATH.
fleet_autonomous_bin() {
    if [ -n "${AUTOSPEC_FLEET_AUTONOMOUS_BIN:-}" ]; then
        printf '%s\n' "$AUTOSPEC_FLEET_AUTONOMOUS_BIN"
        return 0
    fi
    if command -v autospec-autonomous >/dev/null 2>&1; then
        command -v autospec-autonomous
        return 0
    fi
    return 1
}

# ── Fleet worker liveness ─────────────────────────────────────────────────────
# Liveness authority: the same process-heartbeats store every other autospec
# script (heartbeat-write.sh, autospec-watchdog.sh, autospec-run-status.sh)
# reads and writes, keyed by the canonical owner__name slug (repo_slug above).
# Never a PID guess or a `pgrep` on a command string — cross-host and
# cross-restart, a PID means nothing; a heartbeat's freshness does.
#
# fleet-run.sh writes its own small marker file into that same per-repo
# directory the instant a spawn succeeds (fleet_worker_mark_live), so a
# second fleet-run invocation immediately after a spawn sees the repo as
# busy without waiting on the conductor's own first heartbeat write.

fleet_heartbeat_base() {
    printf '%s\n' "${AUTOSPEC_HEARTBEAT_DIR:-${AUTOSPEC_WATCHDOG_DIR:-$HOME/.autospec/process-heartbeats}}"
}

fleet_worker_heartbeat_dir() {
    local normalized="${1:-}"
    local base slug
    base="$(fleet_heartbeat_base)"
    slug="$(repo_slug "$normalized")" || return 2
    printf '%s/%s\n' "${base%/}" "$slug"
}

fleet_worker_heartbeat_file() {
    local normalized="${1:-}"
    local dir
    dir="$(fleet_worker_heartbeat_dir "$normalized")" || return 2
    printf '%s/fleet-worker.json\n' "$dir"
}

# True (0) when a marker for this repo exists and is fresh (mtime within the
# staleness window). A missing or unreadable marker, or a parse/stat failure,
# is treated as "not live" — conservative in the direction of skipping a
# spawn is never safe here, so an unreadable state must not silently block
# a repo forever; it degrades to "attempt a spawn" instead.
fleet_worker_live() {
    local normalized="${1:-}"
    local stale_secs="${AUTOSPEC_FLEET_WORKER_STALE_SECS:-${AUTOSPEC_WATCHDOG_STALE_SECS:-1800}}"
    local hb_file mtime now age

    hb_file="$(fleet_worker_heartbeat_file "$normalized")" || return 1
    [ -f "$hb_file" ] || return 1

    mtime="$(stat -c %Y "$hb_file" 2>/dev/null || stat -f %m "$hb_file" 2>/dev/null || printf '0')"
    case "$mtime" in *[!0-9]*|'') return 1 ;; esac

    now="$(date -u +%s)"
    age=$((now - mtime))
    [ "$age" -lt "$stale_secs" ]
}

# Record that a worker was just started for this repo. Best-effort: callers
# should tolerate a nonzero return (e.g. an unwritable heartbeat dir) rather
# than aborting the fleet over a bookkeeping failure.
fleet_worker_mark_live() {
    local normalized="${1:-}"
    local worker_id="${2:-}"
    local dir hb_file tmp

    dir="$(fleet_worker_heartbeat_dir "$normalized")" || return 2
    mkdir -p "$dir" 2>/dev/null || return 2
    hb_file="$(fleet_worker_heartbeat_file "$normalized")" || return 2
    tmp="${hb_file}.tmp.$$"
    printf '{"repo":"%s","worker_id":"%s","ts":%s}\n' \
        "$normalized" "$worker_id" "$(date -u +%s)" > "$tmp" || return 2
    mv -f "$tmp" "$hb_file" 2>/dev/null || { rm -f "$tmp"; return 2; }
    return 0
}

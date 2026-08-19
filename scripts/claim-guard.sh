#!/usr/bin/env bash
# scripts/claim-guard.sh — atomic file/skill edit-lease with heartbeat-TTL reclaim.
#
# The fine-grained complement to the issue-level claim: the issue claim says
# "I own issue #N"; claim-guard says "I own skills/autospec-explore right now".
# It is the inner layer of the concurrency fix described in
# docs/specs/2026-06-15-claim-guard-concurrent-edit-design.md (Issue A: core).
#
# Subcommands:
#   claim-guard.sh acquire <path|skill>...   atomic all-or-nothing lease.
#                                            0 ok / 6 claim_conflict.
#   claim-guard.sh assert  <path|skill>...   read-only check.
#                                            0 if free/mine, 6 if held by other.
#   claim-guard.sh refresh                   bump updated_at on my claims (heartbeat).
#   claim-guard.sh release [<path|skill>...] drop my claims (default: all mine).
#   claim-guard.sh status                    list live claims for this repo.
#
# `scan` (overlap pre-flight) and the validate.sh gate are Issue B; not here.
#
# Store: ${AUTOSPEC_STATE_DIR:-$HOME/.autospec}/edit-claims/<repo-slug>/<key>.json
#   path-scoped by repo slug subdir (feedback_heartbeat_cross_repo_collision).
#   The on-disk file name is the lock key with ':' and '/' slugified to '-';
#   the canonical lock_key (e.g. skill:autospec-run) lives inside the JSON.
#
# Atomicity: a per-key `.lock` DIR is created with mkdir(2) (POSIX-atomic,
# bash-3.2 safe) before the JSON is written. Multi-path acquire sorts keys and
# takes them in order (deadlock-free), releasing already-taken keys on each
# conflict (all-or-nothing).
#
# Strictness: AUTOSPEC_CLAIM_GUARD=off|warn|strict (default warn).
#   off            -> no-op success, writes nothing.
#   warn (default) -> on conflict, log + proceed (exit 0).
#   strict         -> on conflict, refuse (exit 6).
# An unwritable store always degrades to a no-op success (never blocks work).
#
# Session identity: CLAUDE_CODE_SESSION_ID fallback chain
# (reference_harness_session_id_envs). PPID is the unreliable last resort.
#
# Conventions mirror worktree-guard.sh: usage()/die() helpers, stable
# code_health: identifiers on stderr, no RETURN traps, if/then/fi for one-sided
# conditionals under set -e, bash 3.2 safe.

set -eu

PROG="claim-guard.sh"

STATE_DIR="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}"
MODE="${AUTOSPEC_CLAIM_GUARD:-warn}"
TTL_SECONDS="${AUTOSPEC_CLAIM_TTL_SECONDS:-1800}"

usage() {
    cat <<'EOF'
Usage:
  claim-guard.sh acquire <path|skill>...    atomic all-or-nothing lease (0 ok / 6 conflict)
  claim-guard.sh assert  <path|skill>...    read-only check (0 free/mine / 6 held by other)
  claim-guard.sh refresh                     bump updated_at on my claims (heartbeat)
  claim-guard.sh release [<path|skill>...]   drop my claims (default: all mine in this repo)
  claim-guard.sh status                      list live claims for this repo
  claim-guard.sh scan    <path|skill>...     advisory overlap pre-flight (always exit 0)

Env:
  AUTOSPEC_CLAIM_GUARD   off|warn|strict (default warn). off / unwritable store => no-op.
  AUTOSPEC_STATE_DIR     store root (default ~/.autospec).
  AUTOSPEC_CLAIM_TTL_SECONDS  lease TTL before stale reclaim (default 1800).

Exit codes: 0 ok, 2 usage, 6 claim_conflict.
EOF
}

die() {
    # die <exit-code> <message...>
    code="$1"; shift
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit "$code"
}

emit() { printf '%s\n' "$*" >&2; }

# Telemetry (issue #1774): fire-and-forget `claim` event at acquire/release
# outcomes. Guarded source (absent shim/binary/DSN is a silent no-op) and
# wrapped so nothing here can ever alter this script's exit code — the
# claim lease decision is authoritative, telemetry is best-effort.
emit_claim_event() {
    _cg_conflict="$1"; shift
    _cg_surface="$*"
    # The whole source+emit block is wrapped in `{ ... } || true` (mirroring
    # grow-define-file-issues.sh) so that even a present-but-BROKEN shim —
    # whose `.` source returns non-zero under this script's `set -e` — can
    # never alter the caller's exit code.
    {
        _cg_h="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"
        if [ -f "$_cg_h/emit-event.sh" ]; then
            # shellcheck source=/dev/null
            . "$_cg_h/emit-event.sh"
            emit_event claim surface="$_cg_surface" conflict="$_cg_conflict"
        fi
    } || true
    return 0
}

now_iso() { date -u +'%Y-%m-%dT%H:%M:%SZ'; }
now_epoch() { date -u +%s; }

# Portable ISO8601 -> epoch (BSD date first, GNU fallback) — mirrors
# autospec-run-registry.sh's iso_to_epoch. Unparseable => 0.
iso_to_epoch() {
    ts="$1"
    [ -n "$ts" ] || { echo 0; return; }
    date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$ts" +%s 2>/dev/null \
        || date -u -d "$ts" +%s 2>/dev/null \
        || echo 0
}

# Stable per-session token: first non-empty wins (reference_harness_session_id_envs).
session_id() {
    v=""
    for v in "${AUTOSPEC_SESSION_ID:-}" "${CLAUDE_CODE_SESSION_ID:-}" \
             "${CODEX_SESSION_ID:-}" "${CODEX_THREAD_ID:-}" \
             "${OPENCODE_SESSION_ID:-}" "${OPENCODE_SESSION:-}" \
             "${TERM_SESSION_ID:-}"; do
        if [ -n "$v" ]; then printf '%s' "$v"; return 0; fi
    done
    sid="$(ps -o sess= -p "$$" 2>/dev/null | tr -d ' ')"
    if [ -n "$sid" ] && [ "$sid" != "0" ]; then printf 'sid-%s' "$sid"; return 0; fi
    printf 'ppid-%s' "${PPID:-unknown}"
}

# Source the canonical repo-slug helper (F4): override → sibling (installed
# flat layout) → AUTOSPEC_SCRIPTS_DIR → repo-relative.
_CG_SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
for _rs_cand in \
    "${AUTOSPEC_REPO_SLUG_SH:-}" \
    "${_CG_SELF_DIR}/repo-slug.sh" \
    "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-slug.sh" \
    "${_CG_SELF_DIR}/../scripts/repo-slug.sh"; do
    if [ -n "$_rs_cand" ] && [ -f "$_rs_cand" ]; then
        # shellcheck source=/dev/null
        . "$_rs_cand"
        break
    fi
done

repo_slug() {
    repo="${AUTOSPEC_REPO:-}"
    if [ -z "$repo" ] && command -v gh >/dev/null 2>&1; then
        repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
    fi
    if [ -z "$repo" ]; then
        # Fall back to the git remote URL -> owner/repo.
        url="$(git config --get remote.origin.url 2>/dev/null || true)"
        if [ -n "$url" ]; then
            repo="$(printf '%s' "$url" \
                | sed -e 's#^.*[:/]\([^/]*/[^/]*\)$#\1#' -e 's/\.git$//')"
        fi
    fi
    [ -n "$repo" ] || repo="unknown_repo"
    # Canonical owner__name slug (F4). Reader and writer share this one
    # function, so both migrate together. The slashless "unknown_repo" sentinel
    # has no canonical form and passes through unchanged.
    case "$repo" in
        */*)
            if command -v canonical_slug >/dev/null 2>&1; then
                canonical_slug "$repo"
            else
                printf '%s' "$repo" | sed 's#/#__#'
            fi
            ;;
        *) printf '%s' "$repo" ;;
    esac
}

# Slugify a lock key into a safe file-name stem (':' and '/' -> '-').
key_to_filename() {
    printf '%s' "$1" | tr -c 'A-Za-z0-9_.-' '-'
}

# Resolve an input path/skill token to its canonical lock key.
#   skills/<name>/...                          -> skill:<name>
#   tests/fixtures/skill-goldens/<name>.*      -> skill:<name>
#   <name> when skills/<name>/ exists          -> skill:<name>
#   anything else                              -> path:<normalized>
resolve_key() {
    p="$1"
    # Strip a leading ./ and a trailing slash for normalization.
    p="${p#./}"
    case "$p" in
        skills/*/*|skills/*)
            rest="${p#skills/}"
            name="${rest%%/*}"
            if [ -n "$name" ]; then printf 'skill:%s' "$name"; return 0; fi
            ;;
        tests/fixtures/skill-goldens/*)
            base="${p#tests/fixtures/skill-goldens/}"
            name="${base%%.*}"
            name="${name%%/*}"
            if [ -n "$name" ]; then printf 'skill:%s' "$name"; return 0; fi
            ;;
    esac
    # A bare skill name (no slash) that matches an existing skill dir.
    case "$p" in
        */*) : ;;
        *)
            if [ -n "$p" ] && [ -d "skills/$p" ]; then
                printf 'skill:%s' "$p"; return 0
            fi
            ;;
    esac
    # Normalize a path key: drop trailing slash.
    np="${p%/}"
    printf 'path:%s' "$np"
}

CLAIM_DIR=""           # set by ensure_store
STORE_OK=0             # 1 when the store dir is usable

# Prepare the per-repo store dir. On each failure, STORE_OK stays 0 so callers
# degrade to a no-op (never block work).
ensure_store() {
    slug="$(repo_slug)"
    CLAIM_DIR="${STATE_DIR}/edit-claims/${slug}"
    if mkdir -p "$CLAIM_DIR" 2>/dev/null && [ -w "$CLAIM_DIR" ]; then
        STORE_OK=1
    else
        STORE_OK=0
    fi
}

claim_path()  { printf '%s/%s.json' "$CLAIM_DIR" "$(key_to_filename "$1")"; }
lock_path()   { printf '%s/%s.lock' "$CLAIM_DIR" "$(key_to_filename "$1")"; }

# Is a claim file stale (updated_at older than now - TTL)? A missing/unparseable
# updated_at counts as stale (reclaimable). True (exit 0) == stale.
claim_is_stale() {
    file="$1"
    [ -f "$file" ] || return 0
    updated="$(jq -r '.updated_at // empty' "$file" 2>/dev/null || true)"
    epoch="$(iso_to_epoch "$updated")"
    [ "$epoch" -gt 0 ] || return 0
    age=$(( $(now_epoch) - epoch ))
    [ "$age" -ge "$TTL_SECONDS" ]
}

# Owner session recorded in a claim file ('' if unreadable).
claim_owner() {
    jq -r '.owner_session // empty' "$1" 2>/dev/null || true
}

# Write/refresh a claim JSON for one key held by me.
write_claim() {
    key="$1"; me="$2"; iso="$3"; acquired="$4"
    paths_json="$5"
    cf="$(claim_path "$key")"
    host="${AUTOSPEC_HOST:-$(hostname 2>/dev/null || echo unknown)}"
    branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '')"
    worktree="$(git rev-parse --show-toplevel 2>/dev/null || echo '')"
    tmp="${cf}.tmp.$$"
    # Build the JSON with jq so every field is correctly escaped (branch names,
    # worktree paths, and arbitrary path tokens may contain quotes/backslashes).
    # --argjson injects the already-valid paths[] array verbatim.
    jq -n \
        --arg lock_key "$key" \
        --argjson paths "$paths_json" \
        --arg owner_session "$me" \
        --arg host "$host" \
        --argjson pid "$$" \
        --arg branch "$branch" \
        --arg worktree "$worktree" \
        --arg acquired_at "$acquired" \
        --arg updated_at "$iso" \
        --argjson ttl_seconds "$TTL_SECONDS" \
        '{lock_key:$lock_key, paths:$paths, owner_session:$owner_session,
          host:$host, pid:$pid, branch:$branch, worktree:$worktree,
          acquired_at:$acquired_at, updated_at:$updated_at,
          ttl_seconds:$ttl_seconds}' > "$tmp"
    mv -f "$tmp" "$cf"
}

# A lock dir with NO backing claim file is an orphan from a crash between
# `mkdir <lock>` and `write_claim` (kill -9 window). Treat it as reclaimable
# once it is itself older than the TTL, so a dead acquire never wedges a key
# forever. True (exit 0) == orphaned-and-reclaimable.
lock_is_orphaned() {
    lock="$1"
    [ -d "$lock" ] || return 1
    # Lock dir age via mtime epoch (GNU `stat -c %Y` first, BSD `stat -f %m` fallback).
    mt="$(stat -c %Y "$lock" 2>/dev/null || stat -f %m "$lock" 2>/dev/null || echo 0)"
    [ "$mt" -gt 0 ] || return 1
    age=$(( $(now_epoch) - mt ))
    [ "$age" -ge "$TTL_SECONDS" ]
}

# Take a single key atomically for me. Echoes one of:
#   "acquired"  newly taken (fresh, stale-reclaimed, or orphan-reclaimed)
#   "mine"      I already hold this live claim (idempotent; NOT rolled back)
#   "conflict"  held by a live other session, or a fresh concurrent acquire won
# Never exits — the caller decides the all-or-nothing rollback (only "acquired"
# keys are released on a later conflict; pre-existing "mine" keys are kept).
take_key() {
    key="$1"; me="$2"; iso="$3"; paths_json="$4"
    cf="$(claim_path "$key")"
    lock="$(lock_path "$key")"

    # If a live claim by ANOTHER session exists, that's a conflict regardless of
    # the .lock dir state.
    if [ -f "$cf" ]; then
        owner="$(claim_owner "$cf")"
        if [ -n "$owner" ] && [ "$owner" != "$me" ] && ! claim_is_stale "$cf"; then
            printf 'conflict'
            return 0
        fi
    fi

    # Atomic gate: mkdir the .lock dir. If it already exists, someone else is
    # mid-acquire OR holds the key. Re-check the claim to decide.
    if mkdir "$lock" 2>/dev/null; then
        # We own the lock dir. A stale claim file may still be present; the live
        # check above already proved it is mine or stale, so overwrite is safe.
        acq="$iso"
        if [ -f "$cf" ] && [ "$(claim_owner "$cf")" = "$me" ]; then
            acq="$(jq -r '.acquired_at // empty' "$cf" 2>/dev/null || true)"
            [ -n "$acq" ] || acq="$iso"
        fi
        write_claim "$key" "$me" "$iso" "$acq" "$paths_json"
        printf 'acquired'
        return 0
    fi

    # Could not take the lock dir.
    if [ -f "$cf" ]; then
        owner="$(claim_owner "$cf")"
        if [ "$owner" = "$me" ]; then
            # Idempotent re-acquire of my own live claim: refresh updated_at but
            # do NOT report it as newly acquired (so rollback keeps it).
            acq="$(jq -r '.acquired_at // empty' "$cf" 2>/dev/null || true)"
            [ -n "$acq" ] || acq="$iso"
            write_claim "$key" "$me" "$iso" "$acq" "$paths_json"
            printf 'mine'
            return 0
        fi
        if claim_is_stale "$cf"; then
            # Reclaim. Re-take the lock dir atomically, then RE-VALIDATE that the
            # claim is still stale: the owner may have refreshed in the window
            # between our staleness check and winning the lock. If it went live,
            # back off and report conflict — never steal a now-live lease.
            rmdir "$lock" 2>/dev/null || true
            if mkdir "$lock" 2>/dev/null; then
                if [ -f "$cf" ] \
                        && [ "$(claim_owner "$cf")" != "$me" ] \
                        && ! claim_is_stale "$cf"; then
                    rmdir "$lock" 2>/dev/null || true
                    printf 'conflict'
                    return 0
                fi
                write_claim "$key" "$me" "$iso" "$iso" "$paths_json"
                printf 'acquired'
                return 0
            fi
        fi
        printf 'conflict'
        return 0
    fi

    # Lock dir exists with NO claim file: either a concurrent acquire is
    # mid-flight (it mkdir'd but hasn't written JSON yet — exactly one racer
    # wins, the other lands here as conflict), OR a crashed acquire orphaned the
    # lock. Reclaim only when the lock dir has itself aged past TTL.
    if lock_is_orphaned "$lock"; then
        rmdir "$lock" 2>/dev/null || true
        if mkdir "$lock" 2>/dev/null; then
            # Re-check: another session may have re-taken it concurrently.
            if [ -f "$cf" ] && [ "$(claim_owner "$cf")" != "$me" ] \
                    && ! claim_is_stale "$cf"; then
                rmdir "$lock" 2>/dev/null || true
                printf 'conflict'
                return 0
            fi
            write_claim "$key" "$me" "$iso" "$iso" "$paths_json"
            printf 'acquired'
            return 0
        fi
    fi
    printf 'conflict'
    return 0
}

drop_key() {
    key="$1"
    rm -f "$(claim_path "$key")" 2>/dev/null || true
    rmdir "$(lock_path "$key")" 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# acquire
# ---------------------------------------------------------------------------
cmd_acquire() {
    [ $# -ge 1 ] || die 2 "acquire: at least one <path|skill> is required"
    [ "$MODE" != "off" ] || exit 0
    ensure_store
    [ "$STORE_OK" -eq 1 ] || exit 0   # unwritable store => no-op success

    me="$(session_id)"
    iso="$(now_iso)"

    # Resolve + de-dup keys, sorted for deadlock-free ordering.
    keys="$(for a in "$@"; do resolve_key "$a"; printf '\n'; done | sort -u)"

    # Only keys NEWLY acquired in this invocation are rolled back on a conflict.
    # A key I already held ("mine") is left intact — rolling it back would drop a
    # pre-existing lease the caller still legitimately owns.
    acquired_keys=""
    conflict_key=""
    for k in $keys; do
        # Collect the original input tokens that resolve to this key for the
        # paths[] field (data-model fidelity: paths are source globs, not the
        # key). Build the JSON array from the matching tokens.
        pj="$(
            for a in "$@"; do
                if [ "$(resolve_key "$a")" = "$k" ]; then printf '%s\n' "$a"; fi
            done | jq -R . | jq -s -c .
        )"
        res="$(take_key "$k" "$me" "$iso" "$pj")"
        case "$res" in
            acquired) acquired_keys="$acquired_keys $k" ;;
            mine)     : ;;   # already held by me; keep on rollback
            *)        conflict_key="$k"; break ;;
        esac
    done

    if [ -n "$conflict_key" ]; then
        # all-or-nothing: release only the keys this invocation newly acquired.
        for k in $acquired_keys; do drop_key "$k"; done
        cf="$(claim_path "$conflict_key")"
        owner="$(claim_owner "$cf")"
        host="$(jq -r '.host // empty' "$cf" 2>/dev/null || true)"
        branch="$(jq -r '.branch // empty' "$cf" 2>/dev/null || true)"
        emit "code_health:claim_conflict key=$conflict_key owner_session=$owner host=$host branch=$branch"
        emit_claim_event true "$@"
        if [ "$MODE" = "strict" ]; then
            exit 6
        fi
        # warn mode: log + proceed.
        emit "$PROG: WARN claim conflict on $conflict_key (held by $owner) — proceeding (AUTOSPEC_CLAIM_GUARD=warn)"
        exit 0
    fi

    emit_claim_event false "$@"
    exit 0
}

# ---------------------------------------------------------------------------
# assert (read-only)
# ---------------------------------------------------------------------------
cmd_assert() {
    [ $# -ge 1 ] || die 2 "assert: at least one <path|skill> is required"
    [ "$MODE" != "off" ] || exit 0
    ensure_store
    [ "$STORE_OK" -eq 1 ] || exit 0

    me="$(session_id)"
    keys="$(for a in "$@"; do resolve_key "$a"; printf '\n'; done | sort -u)"
    for k in $keys; do
        cf="$(claim_path "$k")"
        if [ -f "$cf" ]; then
            owner="$(claim_owner "$cf")"
            if [ -n "$owner" ] && [ "$owner" != "$me" ] && ! claim_is_stale "$cf"; then
                host="$(jq -r '.host // empty' "$cf" 2>/dev/null || true)"
                branch="$(jq -r '.branch // empty' "$cf" 2>/dev/null || true)"
                emit "code_health:claim_conflict key=$k owner_session=$owner host=$host branch=$branch"
                if [ "$MODE" = "strict" ]; then exit 6; fi
                emit "$PROG: WARN claim conflict on $k (held by $owner) — proceeding (warn)"
            fi
        fi
    done
    exit 0
}

# ---------------------------------------------------------------------------
# refresh — bump updated_at on every claim owned by me.
# ---------------------------------------------------------------------------
cmd_refresh() {
    [ "$MODE" != "off" ] || exit 0
    ensure_store
    [ "$STORE_OK" -eq 1 ] || exit 0
    me="$(session_id)"
    iso="$(now_iso)"
    if [ -d "$CLAIM_DIR" ]; then
        for cf in "$CLAIM_DIR"/*.json; do
            [ -e "$cf" ] || continue
            owner="$(claim_owner "$cf")"
            if [ "$owner" = "$me" ]; then
                tmp="${cf}.tmp.$$"
                jq --arg u "$iso" '.updated_at = $u' "$cf" > "$tmp" 2>/dev/null \
                    && mv -f "$tmp" "$cf" || rm -f "$tmp"
            fi
        done
    fi
    exit 0
}

# ---------------------------------------------------------------------------
# release — drop my claims (default: all mine; or only the given keys).
# ---------------------------------------------------------------------------
cmd_release() {
    [ "$MODE" != "off" ] || exit 0
    ensure_store
    [ "$STORE_OK" -eq 1 ] || exit 0
    me="$(session_id)"

    if [ $# -ge 1 ]; then
        keys="$(for a in "$@"; do resolve_key "$a"; printf '\n'; done | sort -u)"
        for k in $keys; do
            cf="$(claim_path "$k")"
            if [ -f "$cf" ]; then
                owner="$(claim_owner "$cf")"
                # Only drop a key that is mine (never release another session's).
                if [ -z "$owner" ] || [ "$owner" = "$me" ]; then drop_key "$k"; fi
            else
                drop_key "$k"
            fi
        done
        emit_claim_event false "$@"
        exit 0
    fi

    # No keys -> release ALL claims owned by me in this repo.
    if [ -d "$CLAIM_DIR" ]; then
        for cf in "$CLAIM_DIR"/*.json; do
            [ -e "$cf" ] || continue
            owner="$(claim_owner "$cf")"
            if [ "$owner" = "$me" ]; then
                key="$(jq -r '.lock_key // empty' "$cf" 2>/dev/null || true)"
                rm -f "$cf" 2>/dev/null || true
                if [ -n "$key" ]; then rmdir "$(lock_path "$key")" 2>/dev/null || true; fi
            fi
        done
    fi
    emit_claim_event false all
    exit 0
}

# ---------------------------------------------------------------------------
# status — list live claims for this repo, flagging stale ones.
# ---------------------------------------------------------------------------
cmd_status() {
    ensure_store
    # status is read-only: an unwritable/empty store just prints nothing.
    if [ ! -d "$CLAIM_DIR" ]; then exit 0; fi
    has_claims=0
    for cf in "$CLAIM_DIR"/*.json; do
        [ -e "$cf" ] || continue
        has_claims=1
        key="$(jq -r '.lock_key // empty' "$cf" 2>/dev/null || true)"
        owner="$(jq -r '.owner_session // empty' "$cf" 2>/dev/null || true)"
        host="$(jq -r '.host // empty' "$cf" 2>/dev/null || true)"
        updated="$(jq -r '.updated_at // empty' "$cf" 2>/dev/null || true)"
        flag="live"
        if claim_is_stale "$cf"; then flag="stale"; fi
        printf '%s\towner=%s\thost=%s\tupdated_at=%s\t%s\n' \
            "$key" "$owner" "$host" "$updated" "$flag"
    done
    [ "$has_claims" -eq 1 ] || printf '%s: no live claims for this repo\n' "$PROG"
    exit 0
}

# ---------------------------------------------------------------------------
# scan — advisory pre-flight overlap detection. Always exits 0.
#
# Checks three sources for in-flight edits that overlap the given targets:
#   1. Live claim JSON files in the store (same-filesystem sessions).
#   2. git worktree list --porcelain (other branches on this machine).
#   3. Open PRs via `gh pr list` (degrades gracefully if gh absent/offline).
#
# Emits `code_health:claim_overlap` advisory lines on stderr; never blocks.
# ---------------------------------------------------------------------------
cmd_scan() {
    [ $# -ge 1 ] || die 2 "scan: at least one <path|skill> is required"

    # Resolve target keys.
    target_keys="$(for a in "$@"; do resolve_key "$a"; printf '\n'; done | sort -u)"

    # ------------------------------------------------------------------ #
    # 1. Live claim files: each claim whose lock_key matches a target key  #
    #    and belongs to a different, non-stale session.                   #
    # ------------------------------------------------------------------ #
    ensure_store
    me="$(session_id)"
    if [ "$STORE_OK" -eq 1 ] && [ -d "$CLAIM_DIR" ]; then
        for cf in "$CLAIM_DIR"/*.json; do
            [ -e "$cf" ] || continue
            claim_is_stale "$cf" && continue
            owner="$(claim_owner "$cf")"
            [ -n "$owner" ] || continue
            [ "$owner" = "$me" ] && continue
            held_key="$(jq -r '.lock_key // empty' "$cf" 2>/dev/null || true)"
            [ -n "$held_key" ] || continue
            for tk in $target_keys; do
                if [ "$held_key" = "$tk" ]; then
                    host="$(jq -r '.host // empty' "$cf" 2>/dev/null || true)"
                    branch="$(jq -r '.branch // empty' "$cf" 2>/dev/null || true)"
                    emit "code_health:claim_overlap source=live_claim key=$tk owner_session=$owner host=$host branch=$branch"
                    break
                fi
            done
        done
    fi

    # ------------------------------------------------------------------ #
    # 2. git worktree list: other checked-out branches that are not the   #
    #    current worktree (same-machine; advisory only).                  #
    # ------------------------------------------------------------------ #
    current_wt="$(git rev-parse --show-toplevel 2>/dev/null || true)"
    current_branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
    if command -v git >/dev/null 2>&1; then
        wt_output="$(git worktree list --porcelain 2>/dev/null || true)"
        wt_path=""
        wt_branch=""
        while IFS= read -r line; do
            case "$line" in
                worktree\ *)
                    wt_path="${line#worktree }"
                    wt_branch=""
                    ;;
                branch\ refs/heads/*)
                    wt_branch="${line#branch refs/heads/}"
                    ;;
                "")
                    # End of a worktree stanza — emit advisory if it is another
                    # worktree on a different branch (it may be editing the same
                    # skill; we cannot know without a claim file, so this is
                    # purely informational when the target skill is contested).
                    if [ -n "$wt_path" ] && [ -n "$wt_branch" ] \
                            && [ "$wt_path" != "$current_wt" ] \
                            && [ "$wt_branch" != "$current_branch" ]; then
                        # Only emit if the worktree branch name contains a
                        # fragment of a target key name (heuristic; not a
                        # hard conflict check).
                        for tk in $target_keys; do
                            skill_name="${tk#skill:}"
                            skill_name="${skill_name#path:}"
                            if printf '%s' "$wt_branch" | grep -qF "$skill_name"; then
                                emit "code_health:claim_overlap source=worktree key=$tk worktree=$wt_path branch=$wt_branch"
                                break
                            fi
                        done
                    fi
                    wt_path=""
                    wt_branch=""
                    ;;
            esac
        done <<EOF_WL
$wt_output
EOF_WL
    fi

    # ------------------------------------------------------------------ #
    # 3. Open PRs via gh pr list (degrade cleanly if gh absent/offline). #
    # ------------------------------------------------------------------ #
    if command -v gh >/dev/null 2>&1; then
        pr_json="$(gh pr list --state open --json number,headRefName \
            --limit 50 2>/dev/null || echo '[]')"
        if [ -n "$pr_json" ] && [ "$pr_json" != '[]' ]; then
            pr_count="$(printf '%s' "$pr_json" | jq 'length' 2>/dev/null || echo 0)"
            i=0
            while [ "$i" -lt "$pr_count" ]; do
                pr_branch="$(printf '%s' "$pr_json" | \
                    jq -r ".[$i].headRefName // empty" 2>/dev/null || true)"
                pr_num="$(printf '%s' "$pr_json" | \
                    jq -r ".[$i].number // empty" 2>/dev/null || true)"
                if [ -n "$pr_branch" ]; then
                    for tk in $target_keys; do
                        skill_name="${tk#skill:}"
                        skill_name="${skill_name#path:}"
                        if printf '%s' "$pr_branch" | grep -qF "$skill_name"; then
                            emit "code_health:claim_overlap source=open_pr key=$tk pr=#${pr_num} branch=$pr_branch"
                            break
                        fi
                    done
                fi
                i=$(( i + 1 ))
            done
        fi
    fi

    # scan is always advisory — never block.
    exit 0
}

# ---------------------------------------------------------------------------
# dispatch
# ---------------------------------------------------------------------------
main() {
    [ $# -ge 1 ] || { usage >&2; exit 2; }
    sub="$1"; shift
    case "$sub" in
        acquire)   cmd_acquire "$@" ;;
        assert)    cmd_assert "$@" ;;
        refresh)   cmd_refresh "$@" ;;
        release)   cmd_release "$@" ;;
        status)    cmd_status "$@" ;;
        scan)      cmd_scan "$@" ;;
        -h|--help) usage; exit 0 ;;
        *)         echo "$PROG: unknown subcommand: $sub" >&2; usage >&2; exit 2 ;;
    esac
}

main "$@"

#!/usr/bin/env bash
# Build and publish exactly one immutable Autospec runtime generation.
set -u

SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
# shellcheck source=scripts/autonomous-runtime-refresh.sh
. "$SCRIPT_DIR/autonomous-runtime-refresh.sh"

runtime_install_error() {
    printf 'error:runtime-install:%s\n' "$1" >&2
    return 2
}

runtime_install_setup_dir() {
    if [ -L "$1" ] || { [ -e "$1" ] && [ ! -d "$1" ]; }; then
        runtime_install_error unsafe-state-path
        return 2
    fi
    if [ ! -d "$1" ]; then
        if mkdir "$1" 2>/dev/null; then
            chmod 700 "$1" || return 2
        elif [ ! -d "$1" ] || [ -L "$1" ]; then
            return 2
        fi
    elif [ "$(autospec_runtime_stat_owner "$1")" = "$(id -u)" ]; then
        chmod 700 "$1" || return 2
    fi
    autospec_runtime_private_dir "$1" || { runtime_install_error unsafe-state-directory; return 2; }
}

runtime_process_start() {
    local value
    value=$(ps -o lstart= -p "$1" 2>/dev/null) || return 1
    value=$(printf '%s' "$value" | tr -s ' ' | sed 's/^ //;s/ $//')
    [ -n "$value" ] || return 1
    case "$value" in *$'\n'*|*=*) return 1 ;; esac
    printf '%s\n' "$value"
}

runtime_read_lock() {
    local file line1 line2 line3 extra mode owner
    file=$1
    [ -f "$file" ] && [ ! -L "$file" ] || return 1
    mode=$(autospec_runtime_stat_mode "$file") || return 1
    owner=$(autospec_runtime_stat_owner "$file") || return 1
    [ "$mode" = 600 ] && [ "$owner" = "$(id -u)" ] || return 1
    exec 3<"$file" || return 1
    if ! IFS= read -r line1 <&3 || ! IFS= read -r line2 <&3 || ! IFS= read -r line3 <&3; then
        exec 3<&-
        return 1
    fi
    if IFS= read -r extra <&3 || [ -n "${extra-}" ]; then exec 3<&-; return 1; fi
    exec 3<&-
    case "$line1" in pid=*) lock_pid=${line1#pid=} ;; *) return 1 ;; esac
    case "$line2" in start=*) lock_start=${line2#start=} ;; *) return 1 ;; esac
    case "$line3" in created_at=*) lock_created=${line3#created_at=} ;; *) return 1 ;; esac
    case "$lock_pid" in ''|0|*[!0-9]*) return 1 ;; esac
    [ -n "$lock_start" ] && autospec_runtime_valid_timestamp "$lock_created"
}

runtime_lock_is_live() {
    local actual
    kill -0 "$lock_pid" 2>/dev/null || return 1
    actual=$(runtime_process_start "$lock_pid") || return 1
    [ "$actual" = "$lock_start" ]
}

runtime_reclaim_lock() {
    local recovery="$STATE_ROOT/runtime-install.recovery" abandoned
    if ! mkdir "$recovery" 2>/dev/null; then return 1; fi
    chmod 700 "$recovery" || { rmdir "$recovery"; return 1; }
    if runtime_read_lock "$LOCK_DIR/owner" && ! runtime_lock_is_live; then
        abandoned="$STATE_ROOT/.runtime-install.lock.abandoned.$$"
        if mv "$LOCK_DIR" "$abandoned" 2>/dev/null; then rm -rf "$abandoned"; fi
    fi
    rmdir "$recovery"
}

runtime_acquire_lock() {
    local attempts=0 start temporary
    while ! mkdir "$LOCK_DIR" 2>/dev/null; do
        autospec_runtime_private_dir "$LOCK_DIR" || { runtime_install_error unsafe-lock; return 2; }
        if ! runtime_read_lock "$LOCK_DIR/owner"; then
            attempts=$((attempts + 1))
            if [ "$attempts" -lt 20 ]; then sleep 0.05; continue; fi
            runtime_install_error ambiguous-lock
            return 2
        fi
        if runtime_lock_is_live; then
            attempts=$((attempts + 1))
            [ "$attempts" -lt 1200 ] || { runtime_install_error lock-timeout; return 2; }
            sleep 0.05
            continue
        fi
        runtime_reclaim_lock || { sleep 0.05; continue; }
    done
    chmod 700 "$LOCK_DIR" || return 2
    start=$(runtime_process_start "$$") || { rmdir "$LOCK_DIR"; runtime_install_error process-identity; return 2; }
    temporary="$LOCK_DIR/.owner.$$"
    umask 077
    {
        printf 'pid=%s\n' "$$"
        printf 'start=%s\n' "$start"
        printf 'created_at=%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    } >"$temporary" || return 2
    chmod 600 "$temporary" || return 2
    mv "$temporary" "$LOCK_DIR/owner" || return 2
    LOCK_HELD=1
}

runtime_cleanup() {
    local status=$?
    trap '' HUP INT TERM
    if [ -n "${STAGE_DIR:-}" ] && [ -d "$STAGE_DIR" ]; then chmod -R u+w "$STAGE_DIR" 2>/dev/null || true; rm -rf "$STAGE_DIR"; fi
    if [ "${LOCK_HELD:-0}" -eq 1 ]; then
        rm -f "$JOURNAL"
        rm -rf "$LOCK_DIR"
    fi
    return "$status"
}

runtime_write_journal() {
    local phase=$1 temporary="$STATE_ROOT/.runtime-install.transaction.$$"
    umask 077
    {
        printf 'schema=1\nphase=%s\ndigest=%s\nstage=%s\n' "$phase" "$SOURCE_DIGEST" "$STAGE_DIR"
    } >"$temporary" || return 2
    chmod 600 "$temporary" || return 2
    mv "$temporary" "$JOURNAL" || return 2
}

runtime_recover_interrupted() {
    local stage
    [ ! -e "$JOURNAL" ] && [ ! -L "$JOURNAL" ] && return 0
    [ -f "$JOURNAL" ] && [ ! -L "$JOURNAL" ] || { runtime_install_error unsafe-journal; return 2; }
    [ "$(autospec_runtime_stat_mode "$JOURNAL")" = 600 ] || { runtime_install_error unsafe-journal; return 2; }
    stage=$(sed -n 's/^stage=//p' "$JOURNAL")
    case "$stage" in "$GENERATIONS_ROOT"/.stage.*) [ -d "$stage" ] && rm -rf "$stage" ;; *) runtime_install_error malformed-journal; return 2 ;; esac
    rm -f "$JOURNAL"
}

runtime_publish_pointer() {
    local pointer=$1 target=$2
    if [ -e "$pointer" ] && [ ! -L "$pointer" ]; then runtime_install_error unsafe-current-pointer; return 2; fi
    ln -sfn "$target" "$pointer" || return 2
}

runtime_publish_bin_link() {
    local bin_dir="$STATE_ROOT/bin" pointer="$STATE_ROOT/bin/autospec" temporary="$STATE_ROOT/bin/.autospec.$$"
    runtime_install_setup_dir "$bin_dir" || return 2
    if [ -e "$pointer" ] && [ ! -L "$pointer" ] && [ ! -f "$pointer" ]; then runtime_install_error unsafe-bin-target; return 2; fi
    rm -f "$temporary"
    ln -s '../runtime-generations/current/autospec' "$temporary" || return 2
    mv -f "$temporary" "$pointer" || { rm -f "$temporary"; return 2; }
}

runtime_install_main() {
    local repo='' post_digest built_binary generation receipt
    while [ "$#" -gt 0 ]; do
        case "$1" in --repo-dir) [ "$#" -ge 2 ] && [ -z "$repo" ] || { runtime_install_error usage; return 2; }; repo=$2; shift 2 ;;
            *) runtime_install_error usage; return 2 ;;
        esac
    done
    [ -n "$repo" ] || { runtime_install_error usage; return 2; }
    repo=$(autospec_runtime_repo_dir "$repo") || return 2
    umask 077
    STATE_ROOT="${AUTOSPEC_STATE_ROOT:-$HOME/.autospec}"
    GENERATIONS_ROOT="${AUTOSPEC_RUNTIME_ROOT:-$STATE_ROOT/runtime-generations}"
    LOCK_DIR="$STATE_ROOT/runtime-install.lock"
    JOURNAL="$STATE_ROOT/runtime-install.transaction"
    LOCK_HELD=0; STAGE_DIR=''; SOURCE_DIGEST=''
    runtime_install_setup_dir "$STATE_ROOT" || return 2
    runtime_install_setup_dir "$GENERATIONS_ROOT" || return 2
    runtime_acquire_lock || return 2
    trap runtime_cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 129' HUP
    trap 'exit 143' TERM
    runtime_recover_interrupted || return 2
    SOURCE_DIGEST=$(autospec_runtime_source_digest "$repo") || return 2
    generation="$GENERATIONS_ROOT/$SOURCE_DIGEST"
    if autospec_runtime_verify_generation "$repo" "$SOURCE_DIGEST" "$generation"; then
        runtime_publish_bin_link || return 2
        runtime_publish_pointer "$GENERATIONS_ROOT/current" "$SOURCE_DIGEST" || return 2
        printf '%s/autospec\n' "$generation"
        return 0
    fi
    [ ! -e "$generation" ] && [ ! -L "$generation" ] || { runtime_install_error invalid-existing-generation; return 2; }
    STAGE_DIR="$GENERATIONS_ROOT/.stage.$SOURCE_DIGEST.$$"
    mkdir "$STAGE_DIR" || return 2
    chmod 700 "$STAGE_DIR" || return 2
    runtime_write_journal building || return 2
    CARGO_TARGET_DIR="$STAGE_DIR/target"
    export CARGO_TARGET_DIR
    (CDPATH='' cd -- "$repo" && cargo build --release -p autospec-cli) || { runtime_install_error build-failed; return 2; }
    built_binary="$CARGO_TARGET_DIR/release/autospec"
    [ -f "$built_binary" ] && [ ! -L "$built_binary" ] || { runtime_install_error build-output-missing; return 2; }
    post_digest=$(autospec_runtime_source_digest "$repo") || return 2
    [ "$post_digest" = "$SOURCE_DIGEST" ] || { runtime_install_error source-moved; return 2; }
    mkdir "$STAGE_DIR/generation" || return 2
    chmod 700 "$STAGE_DIR/generation" || return 2
    cp "$built_binary" "$STAGE_DIR/generation/autospec" || return 2
    chmod 500 "$STAGE_DIR/generation/autospec" || return 2
    receipt="$STAGE_DIR/generation/receipt"
    autospec_runtime_write_receipt "$repo" "$STAGE_DIR/generation/autospec" "$receipt" "$SOURCE_DIGEST" || return 2
    autospec_runtime_verify_generation "$repo" "$SOURCE_DIGEST" "$STAGE_DIR/generation" 700 || { runtime_install_error verification-failed; return 2; }
    sync || { runtime_install_error sync-failed; return 2; }
    runtime_write_journal publishing || return 2
    mv "$STAGE_DIR/generation" "$generation" || return 2
    chmod 500 "$generation" || return 2
    autospec_runtime_verify_generation "$repo" "$SOURCE_DIGEST" "$generation" || { runtime_install_error verification-failed; return 2; }
    sync || { runtime_install_error sync-failed; return 2; }
    runtime_publish_bin_link || return 2
    runtime_publish_pointer "$GENERATIONS_ROOT/current" "$SOURCE_DIGEST" || return 2
    rm -rf "$STAGE_DIR"
    STAGE_DIR=''
    printf '%s/autospec\n' "$generation"
}

runtime_install_main "$@"

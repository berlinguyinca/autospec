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

runtime_sync_path() {
    python3 - "$1" <<'PY'
import os, sys
path = sys.argv[1]
flags = os.O_RDONLY
if os.path.isdir(path) and hasattr(os, "O_DIRECTORY"):
    flags |= os.O_DIRECTORY
fd = os.open(path, flags)
try:
    os.fsync(fd)
finally:
    os.close(fd)
PY
}

runtime_atomic_replace() {
    python3 - "$1" "$2" <<'PY'
import os, sys
os.replace(sys.argv[1], sys.argv[2])
parent = os.path.dirname(sys.argv[2]) or "."
flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0)
fd = os.open(parent, flags)
try:
    os.fsync(fd)
finally:
    os.close(fd)
PY
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

runtime_pid_max() {
    local value
    if [ -r /proc/sys/kernel/pid_max ]; then
        IFS= read -r value </proc/sys/kernel/pid_max || return 1
    else
        value=$(sysctl -n kern.pid_max 2>/dev/null) || value=99999
    fi
    case "$value" in ''|0|*[!0-9]*) return 1 ;; esac
    printf '%s\n' "$value"
}

runtime_read_lock() {
    local file line1 line2 line3 extra mode owner pid_max
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
    case "$lock_pid" in ''|0|*[!0-9]*|0*) return 1 ;; esac
    pid_max=$(runtime_pid_max) || return 1
    [ "${#lock_pid}" -le 10 ] && [ "$lock_pid" -le "$pid_max" ] || return 1
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
        [ -e "$LOCK_DIR" ] || continue
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
    if [ -n "${BUILD_DIR:-}" ] && [ -d "$BUILD_DIR" ]; then rm -rf "$BUILD_DIR"; fi
    if [ "${LOCK_HELD:-0}" -eq 1 ]; then
        if [ "${PRESERVE_JOURNAL:-0}" -eq 0 ]; then rm -f "$JOURNAL"; fi
        rm -rf "$LOCK_DIR"
    fi
    return "$status"
}

runtime_write_journal() {
    local phase=$1 temporary="$STATE_ROOT/.runtime-install.transaction.$$"
    umask 077
    {
        printf 'schema=1\nphase=%s\nrepo=%s\nhead=%s\nsource_sha256=%s\ndigest=%s\nstage=%s\nbuild=%s\ndestination=%s\n' \
            "$phase" "$REPO_CANONICAL" "$SOURCE_HEAD" "$SOURCE_SHA" "$SOURCE_DIGEST" "$STAGE_DIR" "$BUILD_DIR" "$DESTINATION"
    } >"$temporary" || return 2
    chmod 600 "$temporary" || return 2
    runtime_sync_path "$temporary" || return 2
    runtime_atomic_replace "$temporary" "$JOURNAL" || return 2
}

runtime_recover_interrupted() {
    local line1 line2 line3 line4 line5 line6 line7 line8 line9 extra schema phase repo head source digest stage build destination journal_owner
    [ ! -e "$JOURNAL" ] && [ ! -L "$JOURNAL" ] && return 0
    PRESERVE_JOURNAL=1
    [ -f "$JOURNAL" ] && [ ! -L "$JOURNAL" ] || { runtime_install_error unsafe-journal; return 2; }
    [ "$(autospec_runtime_stat_mode "$JOURNAL")" = 600 ] || { runtime_install_error unsafe-journal; return 2; }
    journal_owner=$(autospec_runtime_stat_owner "$JOURNAL") || return 2
    [ "$journal_owner" = "$(id -u)" ] || { runtime_install_error unsafe-journal; return 2; }
    exec 3<"$JOURNAL" || return 2
    if ! IFS= read -r line1 <&3 || ! IFS= read -r line2 <&3 || ! IFS= read -r line3 <&3 \
        || ! IFS= read -r line4 <&3 || ! IFS= read -r line5 <&3 || ! IFS= read -r line6 <&3 \
        || ! IFS= read -r line7 <&3 || ! IFS= read -r line8 <&3 || ! IFS= read -r line9 <&3; then exec 3<&-; runtime_install_error malformed-journal; return 2; fi
    if IFS= read -r extra <&3 || [ -n "${extra-}" ]; then exec 3<&-; runtime_install_error malformed-journal; return 2; fi
    exec 3<&-
    case "$line1" in schema=*) schema=${line1#schema=} ;; *) return 2 ;; esac
    case "$line2" in phase=*) phase=${line2#phase=} ;; *) return 2 ;; esac
    case "$line3" in repo=*) repo=${line3#repo=} ;; *) return 2 ;; esac
    case "$line4" in head=*) head=${line4#head=} ;; *) return 2 ;; esac
    case "$line5" in source_sha256=*) source=${line5#source_sha256=} ;; *) return 2 ;; esac
    case "$line6" in digest=*) digest=${line6#digest=} ;; *) return 2 ;; esac
    case "$line7" in stage=*) stage=${line7#stage=} ;; *) return 2 ;; esac
    case "$line8" in build=*) build=${line8#build=} ;; *) return 2 ;; esac
    case "$line9" in destination=*) destination=${line9#destination=} ;; *) return 2 ;; esac
    [ "$schema" = 1 ] || { runtime_install_error malformed-journal; return 2; }
    case "$phase" in building|sealed|published) ;; *) runtime_install_error malformed-journal; return 2 ;; esac
    [ "$repo" = "$(autospec_runtime_repo_dir "$repo" 2>/dev/null)" ] || { runtime_install_error malformed-journal; return 2; }
    [[ $head =~ ^([0-9a-f]{40}|[0-9a-f]{64})$ ]] || { runtime_install_error malformed-journal; return 2; }
    if ! autospec_runtime_valid_sha256 "$source" || ! autospec_runtime_valid_sha256 "$digest"; then
        runtime_install_error malformed-journal
        return 2
    fi
    [ "$stage" = "$GENERATIONS_ROOT/.stage.$digest" ] || { runtime_install_error malformed-journal; return 2; }
    [ "$build" = "$STATE_ROOT/.runtime-build.$digest" ] || { runtime_install_error malformed-journal; return 2; }
    [ "$destination" = "$GENERATIONS_ROOT/$digest" ] || { runtime_install_error malformed-journal; return 2; }
    case "$stage" in "$GENERATIONS_ROOT"/*) ;; *) runtime_install_error malformed-journal; return 2 ;; esac
    case "$destination" in "$GENERATIONS_ROOT"/*) ;; *) runtime_install_error malformed-journal; return 2 ;; esac
    if [ -e "$stage" ] && [ -e "$destination" ]; then runtime_install_error ambiguous-transaction; return 2; fi
    if [ -e "$destination" ]; then
        [ "$phase" != building ] || { runtime_install_error ambiguous-transaction; return 2; }
        autospec_runtime_verify_generation "$repo" "$digest" "$destination" || { runtime_install_error invalid-published-generation; return 2; }
    fi
    [ -d "$stage" ] && { chmod -R u+w "$stage" 2>/dev/null || return 2; rm -rf "$stage" || return 2; }
    [ -d "$build" ] && rm -rf "$build"
    rm -f "$JOURNAL"
    runtime_sync_path "$STATE_ROOT" || return 2
    PRESERVE_JOURNAL=0
}

runtime_publish_pointer() {
    local pointer=$1 target=$2 temporary="$GENERATIONS_ROOT/.current.$$"
    if [ -e "$pointer" ] && [ ! -L "$pointer" ]; then runtime_install_error unsafe-current-pointer; return 2; fi
    rm -f "$temporary"
    ln -s "$target" "$temporary" || return 2
    runtime_atomic_replace "$temporary" "$pointer" || { rm -f "$temporary"; return 2; }
}

runtime_publish_bin_link() {
    local bin_dir="$STATE_ROOT/bin" pointer="$STATE_ROOT/bin/autospec" temporary="$STATE_ROOT/bin/.autospec.$$"
    runtime_install_setup_dir "$bin_dir" || return 2
    if [ -e "$pointer" ] && [ ! -L "$pointer" ] && [ ! -f "$pointer" ]; then runtime_install_error unsafe-bin-target; return 2; fi
    rm -f "$temporary"
    ln -s '../runtime-generations/current/autospec' "$temporary" || return 2
    runtime_atomic_replace "$temporary" "$pointer" || { rm -f "$temporary"; return 2; }
}

runtime_warm_generation() {
    local target generation status
    [ -L "$GENERATIONS_ROOT/current" ] || return 1
    target=$(readlink "$GENERATIONS_ROOT/current") || return 1
    autospec_runtime_valid_sha256 "$target" || return 1
    [ "$target" = "${target##*/}" ] || return 1
    status=$(git -C "$REPO_CANONICAL" status --porcelain --untracked-files=all 2>/dev/null) || return 1
    [ -z "$status" ] || return 1
    generation="$GENERATIONS_ROOT/$target"
    autospec_runtime_parse_receipt "$generation/receipt" || return 1
    [ "$receipt_repo" = "$REPO_CANONICAL" ] && [ "$receipt_head" = "$SOURCE_HEAD" ] \
        && [ "$receipt_identity" = "$target" ] || return 1
    autospec_runtime_verify_generation "$REPO_CANONICAL" "$target" "$generation" || return 1
    runtime_publish_bin_link || return 2
    printf '%s/autospec\n' "$generation"
}

runtime_install_main() {
    local repo='' pre_tuple post_tuple post_repo post_head post_source post_digest built_binary generation receipt
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
    LOCK_HELD=0; PRESERVE_JOURNAL=0; STAGE_DIR=''; BUILD_DIR=''; SOURCE_DIGEST=''
    runtime_install_setup_dir "$STATE_ROOT" || return 2
    runtime_install_setup_dir "$GENERATIONS_ROOT" || return 2
    runtime_acquire_lock || return 2
    trap runtime_cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 129' HUP
    trap 'exit 143' TERM
    runtime_recover_interrupted || return 2
    REPO_CANONICAL=$repo
    SOURCE_HEAD=$(autospec_runtime_head "$repo") || return 2
    if runtime_warm_generation; then return 0; fi
    pre_tuple=$(autospec_runtime_identity_tuple "$repo") || return 2
    REPO_CANONICAL=$(printf '%s\n' "$pre_tuple" | sed -n '1p')
    SOURCE_HEAD=$(printf '%s\n' "$pre_tuple" | sed -n '2p')
    SOURCE_SHA=$(printf '%s\n' "$pre_tuple" | sed -n '3p')
    SOURCE_DIGEST=$(printf '%s\n' "$pre_tuple" | sed -n '4p')
    generation="$GENERATIONS_ROOT/$SOURCE_DIGEST"
    if autospec_runtime_verify_generation "$repo" "$SOURCE_DIGEST" "$generation"; then
        runtime_publish_bin_link || return 2
        runtime_publish_pointer "$GENERATIONS_ROOT/current" "$SOURCE_DIGEST" || return 2
        printf '%s/autospec\n' "$generation"
        return 0
    fi
    [ ! -e "$generation" ] && [ ! -L "$generation" ] || { runtime_install_error invalid-existing-generation; return 2; }
    STAGE_DIR="$GENERATIONS_ROOT/.stage.$SOURCE_DIGEST"
    DESTINATION="$generation"
    mkdir "$STAGE_DIR" || return 2
    chmod 700 "$STAGE_DIR" || return 2
    BUILD_DIR="$STATE_ROOT/.runtime-build.$SOURCE_DIGEST"
    mkdir "$BUILD_DIR" || return 2
    chmod 700 "$BUILD_DIR" || return 2
    runtime_write_journal building || return 2
    CARGO_TARGET_DIR="$BUILD_DIR/target"
    export CARGO_TARGET_DIR
    (CDPATH='' cd -- "$repo" && cargo build --release -p autospec-cli) || { runtime_install_error build-failed; return 2; }
    built_binary="$CARGO_TARGET_DIR/release/autospec"
    [ -f "$built_binary" ] && [ ! -L "$built_binary" ] || { runtime_install_error build-output-missing; return 2; }
    post_tuple=$(autospec_runtime_identity_tuple "$repo") || return 2
    post_repo=$(printf '%s\n' "$post_tuple" | sed -n '1p'); post_head=$(printf '%s\n' "$post_tuple" | sed -n '2p')
    post_source=$(printf '%s\n' "$post_tuple" | sed -n '3p'); post_digest=$(printf '%s\n' "$post_tuple" | sed -n '4p')
    [ "$post_repo" = "$REPO_CANONICAL" ] && [ "$post_head" = "$SOURCE_HEAD" ] \
        && [ "$post_source" = "$SOURCE_SHA" ] && [ "$post_digest" = "$SOURCE_DIGEST" ] \
        || { runtime_install_error source-moved; return 2; }
    cp "$built_binary" "$STAGE_DIR/autospec" || return 2
    chmod 500 "$STAGE_DIR/autospec" || return 2
    receipt="$STAGE_DIR/receipt"
    autospec_runtime_write_receipt "$repo" "$STAGE_DIR/autospec" "$receipt" "$SOURCE_SHA" "$SOURCE_DIGEST" "$SOURCE_HEAD" || return 2
    autospec_runtime_verify_generation "$repo" "$SOURCE_DIGEST" "$STAGE_DIR" 700 || { runtime_install_error verification-failed; return 2; }
    runtime_sync_path "$STAGE_DIR/autospec" || return 2
    runtime_sync_path "$receipt" || return 2
    chmod 400 "$receipt" || return 2
    chmod 500 "$STAGE_DIR" || return 2
    runtime_sync_path "$STAGE_DIR" || return 2
    runtime_write_journal sealed || return 2
    mv "$STAGE_DIR" "$generation" || return 2
    STAGE_DIR=''
    autospec_runtime_verify_generation "$repo" "$SOURCE_DIGEST" "$generation" || { runtime_install_error verification-failed; return 2; }
    runtime_sync_path "$GENERATIONS_ROOT" || return 2
    runtime_write_journal published || return 2
    runtime_publish_bin_link || return 2
    runtime_publish_pointer "$GENERATIONS_ROOT/current" "$SOURCE_DIGEST" || return 2
    rm -rf "$BUILD_DIR"
    BUILD_DIR=''
    printf '%s/autospec\n' "$generation"
}

runtime_install_main "$@"

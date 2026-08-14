#!/usr/bin/env bash
# Build and publish exactly one immutable Autospec runtime generation.
set -u

SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
if ! declare -F autospec_runtime_repo_dir >/dev/null; then
    # shellcheck source=scripts/autonomous-runtime-refresh.sh
    . "$SCRIPT_DIR/autonomous-runtime-refresh.sh"
fi

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

runtime_fast_warm_generation() {
    python3 - "$1" "$2" <<'PY'
import hashlib, os, re, stat, sys
repo_arg, state_root = sys.argv[1:]
def snapshot(repo):
    digest = hashlib.sha256()
    for base, dirs, files in os.walk(repo):
        dirs[:] = sorted(d for d in dirs if d not in (".git", "target"))
        relative_base = os.path.relpath(base, repo)
        for name in dirs + sorted(files):
            path = os.path.join(base, name)
            relative = os.path.normpath(os.path.join(relative_base, name))
            meta = os.lstat(path)
            digest.update(f"{relative}\0{meta.st_mode}\0{meta.st_size}\0{meta.st_mtime_ns}\0{meta.st_ctime_ns}\0".encode())
    return digest.hexdigest()
def git_head(repo):
    dotgit = os.path.join(repo, ".git")
    if os.path.isfile(dotgit):
        with open(dotgit, encoding="utf-8") as handle:
            value = handle.read().strip()
        if not value.startswith("gitdir: "): raise ValueError("gitdir")
        gitdir = os.path.realpath(os.path.join(repo, value[8:]))
    else:
        gitdir = dotgit
    with open(os.path.join(gitdir, "HEAD"), encoding="ascii") as handle:
        value = handle.read().strip()
    if value.startswith("ref: "):
        ref = value[5:]
        ref_path = os.path.join(gitdir, ref)
        if os.path.isfile(ref_path):
            with open(ref_path, encoding="ascii") as handle: return handle.read().strip()
        with open(os.path.join(gitdir, "packed-refs"), encoding="ascii") as handle:
            for line in handle:
                if line.rstrip().endswith(" " + ref): return line.split()[0]
        raise ValueError("ref")
    return value
try:
    repo = os.path.realpath(repo_arg)
    head = git_head(repo)
    root = os.path.join(state_root, "runtime-generations")
    if any(os.path.lexists(os.path.join(state_root, name)) for name in
           ("runtime-install.lock", "runtime-install.transaction", "runtime-install.recovery")):
        raise ValueError("recovery pending")
    current = os.path.join(root, "current")
    digest = os.readlink(current)
    if not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise ValueError("bad pointer")
    generation = os.path.join(root, digest)
    binary, receipt = os.path.join(generation, "autospec"), os.path.join(generation, "receipt")
    for path, mode in ((state_root, 0o700), (root, 0o700), (generation, 0o500),
                       (binary, 0o500), (receipt, 0o400)):
        meta = os.lstat(path)
        if stat.S_IMODE(meta.st_mode) != mode or meta.st_uid != os.getuid() or stat.S_ISLNK(meta.st_mode):
            raise ValueError("unsafe mode")
    with open(receipt, encoding="utf-8") as handle:
        lines = handle.read().splitlines()
    keys = ("schema", "repo_dir", "head", "source_sha256", "identity_sha256",
            "clean_before", "clean_after", "snapshot_sha256", "binary_sha256", "installed_at")
    values = {}
    if len(lines) != len(keys): raise ValueError("receipt shape")
    for key, line in zip(keys, lines):
        prefix = key + "="
        if not line.startswith(prefix): raise ValueError("receipt order")
        values[key] = line[len(prefix):]
    if values["schema"] != "4" or values["repo_dir"] != repo or values["head"] != head:
        raise ValueError("tuple mismatch")
    if values["clean_before"] != "1" or values["clean_after"] != "1":
        raise ValueError("unclean build")
    if values["snapshot_sha256"] != snapshot(repo):
        raise ValueError("snapshot changed")
    identity = hashlib.sha256((f"repo={repo}\0head={head}\0source={values['source_sha256']}\0").encode()).hexdigest()
    if identity != digest or values["identity_sha256"] != digest:
        raise ValueError("identity mismatch")
    with open(binary, "rb") as handle:
        binary_digest = hashlib.file_digest(handle, "sha256").hexdigest() if hasattr(hashlib, "file_digest") else hashlib.sha256(handle.read()).hexdigest()
    if binary_digest != values["binary_sha256"]:
        raise ValueError("binary mismatch")
    print(binary)
except (OSError, ValueError):
    sys.exit(10)
PY
}

runtime_snapshot_digest() {
    python3 - "$1" <<'PY'
import hashlib, os, sys
repo = os.path.realpath(sys.argv[1]); digest = hashlib.sha256()
for base, dirs, files in os.walk(repo):
    dirs[:] = sorted(d for d in dirs if d not in (".git", "target"))
    relative_base = os.path.relpath(base, repo)
    for name in dirs + sorted(files):
        path = os.path.join(base, name); relative = os.path.normpath(os.path.join(relative_base, name)); meta = os.lstat(path)
        digest.update(f"{relative}\0{meta.st_mode}\0{meta.st_size}\0{meta.st_mtime_ns}\0{meta.st_ctime_ns}\0".encode())
print(digest.hexdigest())
PY
}

runtime_rename_exclusive() {
    python3 - "$1" "$2" <<'PY'
import os, sys
os.rename(sys.argv[1], sys.argv[2])
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
    if [ ! -e "$LOCK_DIR/owner" ]; then
        abandoned="$STATE_ROOT/.runtime-install.lock.abandoned.$$"
        if mv "$LOCK_DIR" "$abandoned" 2>/dev/null; then rm -rf "$abandoned"; fi
    elif runtime_read_lock "$LOCK_DIR/owner" && ! runtime_lock_is_live; then
        abandoned="$STATE_ROOT/.runtime-install.lock.abandoned.$$"
        if mv "$LOCK_DIR" "$abandoned" 2>/dev/null; then rm -rf "$abandoned"; fi
    fi
    rmdir "$recovery"
}

runtime_acquire_lock() {
    local attempts=0 start temporary
    ACQUIRE_DIR="$STATE_ROOT/.runtime-install.lock.acquire.$$"
    rm -rf "$ACQUIRE_DIR"
    mkdir "$ACQUIRE_DIR" || return 2
    chmod 700 "$ACQUIRE_DIR" || return 2
    start=$(runtime_process_start "$$") || { runtime_install_error process-identity; return 2; }
    temporary="$ACQUIRE_DIR/owner"
    umask 077
    {
        printf 'pid=%s\n' "$$"
        printf 'start=%s\n' "$start"
        printf 'created_at=%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    } >"$temporary" || return 2
    chmod 600 "$temporary" || return 2
    runtime_sync_path "$temporary" || return 2
    runtime_sync_path "$ACQUIRE_DIR" || return 2
    while ! runtime_rename_exclusive "$ACQUIRE_DIR" "$LOCK_DIR" 2>/dev/null; do
        [ -e "$LOCK_DIR" ] || continue
        if ! autospec_runtime_private_dir "$LOCK_DIR"; then
            [ -e "$LOCK_DIR" ] || continue
            runtime_install_error unsafe-lock
            return 2
        fi
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
    ACQUIRE_DIR=''
    LOCK_HELD=1
}

runtime_cleanup() {
    local status=$?
    trap '' HUP INT TERM
    if [ -n "${STAGE_DIR:-}" ] && [ -d "$STAGE_DIR" ]; then chmod -R u+w "$STAGE_DIR" 2>/dev/null || true; rm -rf "$STAGE_DIR"; fi
    if [ -n "${BUILD_DIR:-}" ] && [ -d "$BUILD_DIR" ]; then rm -rf "$BUILD_DIR"; fi
    if [ -n "${ACQUIRE_DIR:-}" ] && [ -d "$ACQUIRE_DIR" ]; then rm -rf "$ACQUIRE_DIR"; fi
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
    case "$phase" in planned|building|sealed|published) ;; *) runtime_install_error malformed-journal; return 2 ;; esac
    case "$repo" in /*) ;; *) runtime_install_error malformed-journal; return 2 ;; esac
    case "$repo" in *$'\n'*|*$'\r'*|*$'\t'*|*/../*|*/..|*/./*|*/.) runtime_install_error malformed-journal; return 2 ;; esac
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
        case "$phase" in sealed|published) ;; *) runtime_install_error ambiguous-transaction; return 2 ;; esac
        autospec_runtime_verify_recorded_generation "$destination" "$digest" "$repo" "$head" "$source" \
            || { runtime_install_error invalid-published-generation; return 2; }
    elif [ -e "$stage" ] && { [ "$phase" = sealed ] || [ "$phase" = published ]; }; then
        autospec_runtime_verify_recorded_generation "$stage" "$digest" "$repo" "$head" "$source" \
            || { runtime_install_error invalid-sealed-generation; return 2; }
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
    if [ -L "$pointer" ] && [ "$(readlink "$pointer" 2>/dev/null)" = '../runtime-generations/current/autospec' ]; then return 0; fi
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
        && [ "$receipt_identity" = "$target" ] && [ "$receipt_clean_before" = 1 ] \
        && [ "$receipt_clean_after" = 1 ] || return 1
    autospec_runtime_verify_generation "$REPO_CANONICAL" "$target" "$generation" || return 1
    printf '%s/autospec\n' "$generation"
}

runtime_install_main() {
    local repo='' check_only=0 pre_tuple post_tuple post_repo post_head post_source post_digest snapshot built_binary generation receipt
    while [ "$#" -gt 0 ]; do
        case "$1" in --repo-dir) [ "$#" -ge 2 ] && [ -z "$repo" ] || { runtime_install_error usage; return 2; }; repo=$2; shift 2 ;;
            --check-only) check_only=1; shift ;;
            *) runtime_install_error usage; return 2 ;;
        esac
    done
    [ -n "$repo" ] || { runtime_install_error usage; return 2; }
    STATE_ROOT="${AUTOSPEC_STATE_ROOT:-$HOME/.autospec}"
    if runtime_fast_warm_generation "$repo" "$STATE_ROOT"; then return 0; fi
    if [ "$check_only" -eq 1 ]; then printf 'stale:generation-invalid\n'; return 10; fi
    repo=$(autospec_runtime_repo_dir "$repo") || return 2
    umask 077
    GENERATIONS_ROOT="${AUTOSPEC_RUNTIME_ROOT:-$STATE_ROOT/runtime-generations}"
    LOCK_DIR="$STATE_ROOT/runtime-install.lock"
    JOURNAL="$STATE_ROOT/runtime-install.transaction"
    LOCK_HELD=0; PRESERVE_JOURNAL=0; STAGE_DIR=''; BUILD_DIR=''; ACQUIRE_DIR=''; SOURCE_DIGEST=''
    runtime_install_setup_dir "$STATE_ROOT" || return 2
    runtime_install_setup_dir "$GENERATIONS_ROOT" || return 2
    REPO_CANONICAL=$repo
    SOURCE_HEAD=$(autospec_runtime_head "$repo") || return 2
    if [ ! -e "$LOCK_DIR" ] && [ ! -e "$JOURNAL" ] && [ ! -e "$STATE_ROOT/runtime-install.recovery" ]; then
        if runtime_warm_generation; then return 0; fi
    fi
    trap runtime_cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 129' HUP
    trap 'exit 143' TERM
    runtime_acquire_lock || return 2
    runtime_recover_interrupted || return 2
    SOURCE_HEAD=$(autospec_runtime_head "$repo") || return 2
    if runtime_warm_generation; then return 0; fi
    pre_tuple=$(autospec_runtime_identity_tuple "$repo") || return 2
    REPO_CANONICAL=$(printf '%s\n' "$pre_tuple" | sed -n '1p')
    SOURCE_HEAD=$(printf '%s\n' "$pre_tuple" | sed -n '2p')
    SOURCE_SHA=$(printf '%s\n' "$pre_tuple" | sed -n '3p')
    SOURCE_DIGEST=$(printf '%s\n' "$pre_tuple" | sed -n '4p')
    if [ -z "$(git -C "$repo" status --porcelain --untracked-files=all 2>/dev/null)" ]; then CLEAN_BEFORE=1; else CLEAN_BEFORE=0; fi
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
    BUILD_DIR="$STATE_ROOT/.runtime-build.$SOURCE_DIGEST"
    runtime_write_journal planned || return 2
    mkdir "$STAGE_DIR" || return 2
    chmod 700 "$STAGE_DIR" || return 2
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
    if [ -z "$(git -C "$repo" status --porcelain --untracked-files=all 2>/dev/null)" ]; then CLEAN_AFTER=1; else CLEAN_AFTER=0; fi
    if [ "$CLEAN_BEFORE" = 1 ] && [ "$CLEAN_AFTER" = 1 ]; then snapshot=$(runtime_snapshot_digest "$repo") || return 2; else snapshot=$(printf '%064d' 0); fi
    [ "$post_repo" = "$REPO_CANONICAL" ] && [ "$post_head" = "$SOURCE_HEAD" ] \
        && [ "$post_source" = "$SOURCE_SHA" ] && [ "$post_digest" = "$SOURCE_DIGEST" ] \
        || { runtime_install_error source-moved; return 2; }
    cp "$built_binary" "$STAGE_DIR/autospec" || return 2
    chmod 500 "$STAGE_DIR/autospec" || return 2
    receipt="$STAGE_DIR/receipt"
    autospec_runtime_write_receipt "$repo" "$STAGE_DIR/autospec" "$receipt" "$SOURCE_SHA" "$SOURCE_DIGEST" "$SOURCE_HEAD" "$CLEAN_BEFORE" "$CLEAN_AFTER" "$snapshot" || return 2
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

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    runtime_install_main "$@"
fi

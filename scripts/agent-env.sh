#!/usr/bin/env bash
# agent-env.sh — manifest-driven isolated runtime broker for agent harnesses.

set -eu

COMMAND="${1:-}"
[ -n "$COMMAND" ] && shift || true

REPO="."
MODE="auto"

die() {
    printf 'agent-env: %s\n' "$*" >&2
    exit 1
}

missing_manifest() {
    printf 'agent-env: no runtime manifest found in %s (.autospec/runtime.yml or .agent-runtime.yml)\n' "$1" >&2
    exit 2
}

usage() {
    cat <<'EOF'
Usage:
  agent-env up [--repo PATH] [--mode MODE]
  agent-env status [--repo PATH] [--mode MODE]
  agent-env down [--repo PATH] [--mode MODE]
  agent-env exec [--repo PATH] [--mode MODE] -- COMMAND [ARGS...]
  agent-env session [--repo PATH] [--mode MODE] [--keep-alive] -- COMMAND [ARGS...]

Reads .autospec/runtime.yml or .agent-runtime.yml and exports a per-repo
runtime environment with dynamic ports, AUTOSPEC_PUBLIC_URL, and AGENT_PUBLIC_URL.
EOF
}

parse_common_args() {
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo)
                [ "$#" -ge 2 ] || die "missing value for --repo"
                REPO="$2"; shift 2 ;;
            --repo=*)
                REPO="${1#--repo=}"; shift ;;
            --mode)
                [ "$#" -ge 2 ] || die "missing value for --mode"
                MODE="$2"; shift 2 ;;
            --mode=*)
                MODE="${1#--mode=}"; shift ;;
            --)
                shift
                break ;;
            -*)
                die "unknown option: $1" ;;
            *)
                break ;;
        esac
    done
    REMAINING_ARGS="$*"
}

repo_realpath() {
    (cd "$1" 2>/dev/null && pwd -P) || die "repo does not exist: $1"
}

manifest_path() {
    repo="$1"
    if find_manifest_path "$repo"; then
        return 0
    fi
    missing_manifest "$repo"
}

find_manifest_path() {
    repo="$1"
    if [ -f "$repo/.autospec/runtime.yml" ]; then
        printf '%s\n' "$repo/.autospec/runtime.yml"
    elif [ -f "$repo/.agent-runtime.yml" ]; then
        printf '%s\n' "$repo/.agent-runtime.yml"
    else
        return 1
    fi
}

yaml_scalar() {
    key="$1"; file="$2"
    awk -v key="$key" '
        $0 ~ "^[[:space:]]*" key ":[[:space:]]*" {
            sub("^[[:space:]]*" key ":[[:space:]]*", "")
            gsub(/^["'\'']|["'\'']$/, "")
            print
            exit
        }
    ' "$file"
}

yaml_mode_field() {
    file="$1"; mode="$2"; field="$3"
    awk -v mode="$mode" -v field="$field" '
        /^  [^ ].*:[[:space:]]*$/ {
            line=$0
            sub(/^  /, "", line)
            sub(/:[[:space:]]*$/, "", line)
            in_mode=(line == mode)
            next
        }
        in_mode && $0 ~ "^    " field ":[[:space:]]*" {
            sub("^[[:space:]]*" field ":[[:space:]]*", "")
            print
            exit
        }
    ' "$file"
}

yaml_mode_env_pairs() {
    file="$1"; mode="$2"
    awk -v mode="$mode" '
        /^  [^ ].*:[[:space:]]*$/ {
            line=$0
            sub(/^  /, "", line)
            sub(/:[[:space:]]*$/, "", line)
            in_mode=(line == mode)
            in_env=0
            next
        }
        in_mode && /^    env:[[:space:]]*$/ { in_env=1; next }
        in_mode && in_env && /^    [^ ].*:/ { exit }
        in_mode && in_env && /^      [A-Za-z_][A-Za-z0-9_]*:[[:space:]]*/ {
            line=$0
            sub(/^      /, "", line)
            key=line
            sub(/:.*/, "", key)
            sub(/^[^:]*:[[:space:]]*/, "", line)
            gsub(/^["'\'']|["'\'']$/, "", line)
            print key "=" line
        }
    ' "$file"
}

first_mode() {
    file="$1"
    awk '
        /^modes:[[:space:]]*$/ { in_modes=1; next }
        in_modes && /^  [^ ].*:[[:space:]]*$/ {
            line=$0
            sub(/^  /, "", line)
            sub(/:[[:space:]]*$/, "", line)
            print line
            exit
        }
    ' "$file"
}

slugify() {
    tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9_-]/_/g; s/__*/_/g; s/^_//; s/_$//'
}

compose_slugify() {
    tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9_]/_/g; s/__*/_/g; s/^_//; s/_$//'
}

short_hash() {
    cksum | awk '{print $1}'
}

free_port() {
    if command -v python3 >/dev/null 2>&1; then
        python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
    else
        awk 'BEGIN { srand(); print int(20000 + rand() * 30000) }'
    fi
}

shell_quote() {
    # POSIX-safe single-quote escaping for env files.
    printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

write_env_file() {
    env_file="$1"
    {
        printf 'export AGENT_ENV_ID=%s\n' "$(shell_quote "$AGENT_ENV_ID")"
        printf 'export AGENT_ENV_MODE=%s\n' "$(shell_quote "$AGENT_ENV_MODE")"
        printf 'export AGENT_ENV_REPO=%s\n' "$(shell_quote "$AGENT_ENV_REPO")"
        printf 'export AGENT_ENV_MANIFEST=%s\n' "$(shell_quote "$AGENT_ENV_MANIFEST")"
        printf 'export AGENT_FRONTEND_PORT=%s\n' "$(shell_quote "$AGENT_FRONTEND_PORT")"
        printf 'export AGENT_BACKEND_PORT=%s\n' "$(shell_quote "$AGENT_BACKEND_PORT")"
        printf 'export AGENT_PUBLIC_URL=%s\n' "$(shell_quote "$AGENT_PUBLIC_URL")"
        printf 'export AUTOSPEC_PUBLIC_URL=%s\n' "$(shell_quote "$AUTOSPEC_PUBLIC_URL")"
        printf 'export COMPOSE_PROJECT_NAME=%s\n' "$(shell_quote "$COMPOSE_PROJECT_NAME")"
        yaml_mode_env_pairs "$AGENT_ENV_MANIFEST" "$AGENT_ENV_MODE" | while IFS='=' read -r key value; do
            [ -n "$key" ] || continue
            printf 'export %s=%s\n' "$key" "$(shell_quote "$value")"
        done
    } > "$env_file"
}

print_env() {
    printf 'AGENT_ENV_ID=%s\n' "$AGENT_ENV_ID"
    printf 'AGENT_ENV_MODE=%s\n' "$AGENT_ENV_MODE"
    printf 'AGENT_ENV_REPO=%s\n' "$AGENT_ENV_REPO"
    printf 'AGENT_ENV_FILE=%s\n' "$AGENT_ENV_FILE"
    printf 'AGENT_FRONTEND_PORT=%s\n' "$AGENT_FRONTEND_PORT"
    printf 'AGENT_BACKEND_PORT=%s\n' "$AGENT_BACKEND_PORT"
    printf 'AGENT_PUBLIC_URL=%s\n' "$AGENT_PUBLIC_URL"
    printf 'AUTOSPEC_PUBLIC_URL=%s\n' "$AUTOSPEC_PUBLIC_URL"
    printf 'COMPOSE_PROJECT_NAME=%s\n' "$COMPOSE_PROJECT_NAME"
    yaml_mode_env_pairs "$AGENT_ENV_MANIFEST" "$AGENT_ENV_MODE" | while IFS='=' read -r key value; do
        [ -n "$key" ] || continue
        printf '%s=%s\n' "$key" "$value"
    done
}

load_context() {
    AGENT_ENV_REPO="$(repo_realpath "$REPO")"
    AGENT_ENV_MANIFEST="$(manifest_path "$AGENT_ENV_REPO")"
    manifest_name="$(yaml_scalar name "$AGENT_ENV_MANIFEST")"
    [ -n "$manifest_name" ] || manifest_name="$(basename "$AGENT_ENV_REPO")"
    if [ "$MODE" = "auto" ]; then
        MODE="$(yaml_scalar default_mode "$AGENT_ENV_MANIFEST")"
        [ -n "$MODE" ] || MODE="$(first_mode "$AGENT_ENV_MANIFEST")"
    fi
    [ -n "$MODE" ] || die "runtime manifest has no selectable mode"
    AGENT_ENV_MODE="$MODE"

    hash="$(printf '%s' "$AGENT_ENV_REPO:$AGENT_ENV_MODE" | short_hash)"
    name_slug="$(printf '%s' "$manifest_name" | slugify)"
    [ -n "$name_slug" ] || name_slug="agent_env"
    AGENT_ENV_ID="${name_slug}-${hash}"
    state_root="${AGENT_ENV_STATE_ROOT:-$HOME/.autospec/envs}"
    AGENT_ENV_DIR="$state_root/$AGENT_ENV_ID"
    AGENT_ENV_FILE="$AGENT_ENV_DIR/env"
}

prepare_env() {
    mkdir -p "$AGENT_ENV_DIR"
    AGENT_FRONTEND_PORT="${AGENT_FRONTEND_PORT:-$(free_port)}"
    AGENT_BACKEND_PORT="${AGENT_BACKEND_PORT:-$(free_port)}"
    AGENT_PUBLIC_URL="${AGENT_PUBLIC_URL:-http://127.0.0.1:$AGENT_FRONTEND_PORT}"
    AUTOSPEC_PUBLIC_URL="${AUTOSPEC_PUBLIC_URL:-$AGENT_PUBLIC_URL}"
    compose_slug="$(printf '%s' "$AGENT_ENV_ID" | compose_slugify)"
    COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-agent_$compose_slug}"
    export AGENT_ENV_ID AGENT_ENV_MODE AGENT_ENV_REPO AGENT_ENV_MANIFEST
    export AGENT_FRONTEND_PORT AGENT_BACKEND_PORT AGENT_PUBLIC_URL AUTOSPEC_PUBLIC_URL
    export COMPOSE_PROJECT_NAME
    write_env_file "$AGENT_ENV_FILE"
    # shellcheck disable=SC1090
    . "$AGENT_ENV_FILE"
}

run_manifest_command() {
    field="$1"
    cmd="$(yaml_mode_field "$AGENT_ENV_MANIFEST" "$AGENT_ENV_MODE" "$field")"
    [ -n "$cmd" ] || return 3
    (cd "$AGENT_ENV_REPO" && sh -c "$cmd")
}

start_current_env() {
    if [ -f "$AGENT_ENV_FILE" ]; then
        # shellcheck disable=SC1090
        . "$AGENT_ENV_FILE"
        return 0
    fi
    prepare_env
    if run_manifest_command command; then
        :
    else
        rc=$?
        [ "$rc" -eq 3 ] && die "mode '$AGENT_ENV_MODE' has no command in $AGENT_ENV_MANIFEST"
        exit "$rc"
    fi
}

down_current_env() {
    if [ -f "$AGENT_ENV_FILE" ]; then
        # shellcheck disable=SC1090
        . "$AGENT_ENV_FILE"
    fi
    run_manifest_command down || {
        rc=$?
        [ "$rc" -eq 3 ] || exit "$rc"
    }
    rm -rf "$AGENT_ENV_DIR"
}

cmd_up() {
    parse_common_args "$@"
    load_context
    start_current_env
    print_env
}

cmd_status() {
    parse_common_args "$@"
    load_context
    if [ ! -f "$AGENT_ENV_FILE" ]; then
        printf 'agent-env: no active environment for %s mode %s\n' "$AGENT_ENV_REPO" "$AGENT_ENV_MODE" >&2
        exit 3
    fi
    # shellcheck disable=SC1090
    . "$AGENT_ENV_FILE"
    print_env
}

cmd_down() {
    parse_common_args "$@"
    load_context
    down_current_env
}

cmd_exec() {
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo)
                [ "$#" -ge 2 ] || die "missing value for --repo"
                REPO="$2"; shift 2 ;;
            --repo=*)
                REPO="${1#--repo=}"; shift ;;
            --mode)
                [ "$#" -ge 2 ] || die "missing value for --mode"
                MODE="$2"; shift 2 ;;
            --mode=*)
                MODE="${1#--mode=}"; shift ;;
            --)
                shift
                break ;;
            -*)
                die "unknown option: $1" ;;
            *)
                break ;;
        esac
    done
    [ "$#" -gt 0 ] || die "exec requires a command after --"
    load_context
    start_current_env
    (cd "$AGENT_ENV_REPO" && "$@")
}

cmd_session() {
    keep_alive="${AUTOSPEC_ENV_KEEP_ALIVE:-0}"
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo)
                [ "$#" -ge 2 ] || die "missing value for --repo"
                REPO="$2"; shift 2 ;;
            --repo=*)
                REPO="${1#--repo=}"; shift ;;
            --mode)
                [ "$#" -ge 2 ] || die "missing value for --mode"
                MODE="$2"; shift 2 ;;
            --mode=*)
                MODE="${1#--mode=}"; shift ;;
            --keep-alive)
                keep_alive=1; shift ;;
            --)
                shift
                break ;;
            -*)
                die "unknown option: $1" ;;
            *)
                break ;;
        esac
    done
    [ "$#" -gt 0 ] || die "session requires a command after --"

    AGENT_ENV_REPO="$(repo_realpath "$REPO")"
    if [ "${AUTOSPEC_ENV_DISABLE:-0}" = "1" ] || ! find_manifest_path "$AGENT_ENV_REPO" >/dev/null 2>&1; then
        (cd "$AGENT_ENV_REPO" && "$@")
        exit "$?"
    fi

    load_context
    start_current_env

    session_dir="$AGENT_ENV_DIR/sessions"
    mkdir -p "$session_dir"
    session_file="$session_dir/$$"
    {
        printf 'pid=%s\n' "$$"
        printf 'command=%s\n' "$*"
        printf 'started_at=%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date)"
    } > "$session_file"

    child_pid=""
    cleanup() {
        rc="${1:-0}"
        rm -f "$session_file"
        if [ "$keep_alive" != "1" ]; then
            down_current_env
        fi
        exit "$rc"
    }
    trap 'if [ -n "$child_pid" ]; then kill "$child_pid" 2>/dev/null || true; fi; cleanup 130' INT
    trap 'if [ -n "$child_pid" ]; then kill "$child_pid" 2>/dev/null || true; fi; cleanup 143' TERM

    set +e
    (cd "$AGENT_ENV_REPO" && "$@") &
    child_pid=$!
    wait "$child_pid"
    rc=$?
    child_pid=""
    set -e
    cleanup "$rc"
}

case "$COMMAND" in
    up) cmd_up "$@" ;;
    status) cmd_status "$@" ;;
    down) cmd_down "$@" ;;
    exec) cmd_exec "$@" ;;
    session) cmd_session "$@" ;;
    -h|--help|help|"") usage ;;
    *) die "unknown command: $COMMAND" ;;
esac

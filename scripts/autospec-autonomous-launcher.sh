#!/usr/bin/env bash
# Fail-closed runtime freshness boundary for autonomous operator wrappers.
set -u

launcher_error() {
    printf 'error:autospec-autonomous-launcher:%s\n' "$1" >&2
    return 2
}

launcher_is_start_family() {
    case "${1-}" in
        ''|start|restart|-*) return 0 ;;
        *) return 1 ;;
    esac
}

launcher_repo_dir() {
    local previous='' argument
    for argument in "$@"; do
        if [ "$previous" = repo-dir ]; then
            [ -n "$argument" ] || return 1
            printf '%s\n' "$argument"
            return 0
        fi
        case "$argument" in
            --repo-dir) previous=repo-dir ;;
            --repo-dir=*)
                argument=${argument#--repo-dir=}
                [ -n "$argument" ] || return 1
                printf '%s\n' "$argument"
                return 0
                ;;
            *) previous='' ;;
        esac
    done
    [ "$previous" != repo-dir ] || return 1
    git rev-parse --show-toplevel 2>/dev/null
}

launcher_runtime_path() {
    local path=$1
    [ -n "$path" ] || return 1
    case "$path" in
        /*) ;;
        *) return 1 ;;
    esac
    case "$path" in *$'\n'*|*$'\r'*) return 1 ;; esac
    [ -f "$path" ] && [ ! -L "$path" ] && [ -x "$path" ]
}

launcher_status() {
    local repo_dir=$1 output
    output=$(autospec autonomous status --repo-dir "$repo_dir" --json 2>/dev/null) || return 2
    case "$output" in
        *'"metadata_state":"ambiguous"'*) return 2 ;;
        *'"running":true'*) return 10 ;;
        *'"running":false'*) return 0 ;;
        *) return 2 ;;
    esac
}

launcher_wait_for_stopped_scope() {
    local repo_dir=$1 attempts=0 status
    while [ "$attempts" -lt 600 ]; do
        if launcher_status "$repo_dir"; then
            return 0
        else
            status=$?
        fi
        [ "$status" -eq 10 ] || {
            launcher_error 'cannot verify whether the stale autonomous scope is stopped'
            return 2
        }
        attempts=$((attempts + 1))
        sleep 0.1
    done
    launcher_error 'timed out waiting for the stale autonomous scope to stop'
}

launcher_drain_if_live() {
    local repo_dir=$1 status
    command -v autospec >/dev/null 2>&1 || {
        launcher_error 'cannot verify stale autonomous scope because autospec is unavailable'
        return 2
    }
    if launcher_status "$repo_dir"; then
        return 0
    else
        status=$?
    fi
    [ "$status" -eq 10 ] || {
        launcher_error 'cannot verify whether the stale autonomous scope is stopped'
        return 2
    }
    autospec autonomous stop --repo-dir "$repo_dir" --graceful >/dev/null || {
        launcher_error 'could not request graceful stop for the stale autonomous scope'
        return 2
    }
    launcher_wait_for_stopped_scope "$repo_dir"
}

launcher_exec_installed() {
    command -v autospec >/dev/null 2>&1 || {
        launcher_error 'autospec Rust binary is required; install it before using autonomous commands'
        return 127
    }
    exec autospec autonomous "$@"
}

launcher_main() {
    local script_dir helper repo_dir runtime_repo_dir runtime_path check_status
    if ! launcher_is_start_family "${1-}"; then
        launcher_exec_installed "$@"
    fi

    script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd) || return 2
    helper="$script_dir/autonomous-runtime-refresh.sh"
    [ -f "$helper" ] && [ -x "$helper" ] || {
        launcher_error "runtime refresh helper is required at $helper"
        return 2
    }
    repo_dir=$(launcher_repo_dir "$@") || {
        launcher_error 'cannot resolve --repo-dir or the caller git root'
        return 2
    }
    runtime_repo_dir=$(bash "$helper" source --repo-dir "$repo_dir") || {
        launcher_error 'cannot resolve the installed Autospec source checkout'
        return 2
    }

    if runtime_path=$(bash "$helper" check --repo-dir "$runtime_repo_dir"); then
        launcher_runtime_path "$runtime_path" || {
            launcher_error 'runtime freshness check returned an invalid executable path'
            return 2
        }
    else
        check_status=$?
        [ "$check_status" -eq 10 ] || {
            launcher_error 'runtime freshness check failed'
            return 2
        }
        launcher_drain_if_live "$repo_dir" || return $?
        runtime_path=$(bash "$helper" ensure --repo-dir "$runtime_repo_dir") || {
            launcher_error 'could not publish a fresh autonomous runtime'
            return 2
        }
        launcher_runtime_path "$runtime_path" || {
            launcher_error 'runtime refresh returned an invalid executable path'
            return 2
        }
    fi

    exec "$runtime_path" autonomous "$@"
}

launcher_main "$@"

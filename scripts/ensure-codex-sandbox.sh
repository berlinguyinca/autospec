#!/usr/bin/env bash
# Repair the Ubuntu AppArmor restriction that prevents Codex permission
# profiles from creating Bubblewrap user namespaces.

set -eu

PROFILE_NAME="autospec-network-executor"
HOST_ERROR="codex_sandbox_host_setup_required"
UNAVAILABLE_ERROR="codex_sandbox_unavailable"
TEST_ROOT_ERROR="codex_sandbox_test_root_refused"
SYSTEM_ROOT="${AUTOSPEC_CODEX_SANDBOX_ROOT:-}"
TEST_MODE="${AUTOSPEC_CODEX_SANDBOX_TEST_MODE:-0}"
CODEX_EXECUTABLE=""
CODEX_EXECUTABLE_TOML=""
CODEX_VENDOR_READ_TOML=""
ORIGINAL_CODEX_HOME="${CODEX_HOME:-}"
PROBE_CODEX_HOME=""
IGNORE_USER_CONFIG_SUPPORTED=0
CLEAN_BOUNDARY_PATH="/usr/sbin:/usr/bin:/sbin:/bin"

error() {
    printf 'error: %s\n' "$*" >&2
}

cleanup() {
    [ -z "$PROBE_CODEX_HOME" ] || rm -rf -- "$PROBE_CODEX_HOME"
}
trap cleanup EXIT INT TERM

run_clean_shell() {
    clean_script=$1
    shift
    /usr/bin/env -i \
        "PATH=$CLEAN_BOUNDARY_PATH" \
        /bin/bash --noprofile --norc -p -c "$clean_script" \
        autospec-codex-sandbox "$@"
}

if [ -n "$SYSTEM_ROOT" ]; then
    if [ "$TEST_MODE" != "1" ]; then
        error "$TEST_ROOT_ERROR: AUTOSPEC_CODEX_SANDBOX_ROOT is allowed only with AUTOSPEC_CODEX_SANDBOX_TEST_MODE=1"
        exit 1
    fi
    case "$SYSTEM_ROOT" in
        /*) SYSTEM_ROOT="${SYSTEM_ROOT%/}" ;;
        *)
            error "$TEST_ROOT_ERROR: AUTOSPEC_CODEX_SANDBOX_ROOT must be absolute"
            exit 1
            ;;
    esac
elif [ "$TEST_MODE" = "1" ]; then
    error "$TEST_ROOT_ERROR: test mode requires an isolated AUTOSPEC_CODEX_SANDBOX_ROOT"
    exit 1
fi

OS_RELEASE="${SYSTEM_ROOT}/etc/os-release"
PROFILE_DIR="${SYSTEM_ROOT}/etc/apparmor.d"
PROFILE_PATH="$PROFILE_DIR/usr.bin.bwrap"

if [ "${1:-}" = "--test-clean-boundary" ]; then
    if [ "$TEST_MODE" != "1" ]; then
        error "$TEST_ROOT_ERROR: clean-boundary verification is test-only"
        exit 1
    fi
    # shellcheck disable=SC2016 # Assertions expand only inside the clean shell.
    run_clean_shell '
set -eu
[ "$PATH" = /usr/sbin:/usr/bin:/sbin:/bin ]
[ -z "${BASH_ENV:-}" ]
[ -z "${AUTOSPEC_BASH_ENV_LOADED:-}" ]
! command -v autospec-boundary-attack >/dev/null 2>&1
printf "codex_sandbox_clean_boundary:verified\n"
'
    exit
fi

is_ubuntu() {
    [ -r "$OS_RELEASE" ] || return 1
    while IFS='=' read -r key value; do
        if [ "$key" = "ID" ]; then
            value="${value#\"}"
            value="${value%\"}"
            [ "$value" = "ubuntu" ]
            return
        fi
    done < "$OS_RELEASE"
    return 1
}

apparmor_restricts_userns() {
    command -v sysctl >/dev/null 2>&1 || return 1
    [ "$(sysctl -n kernel.apparmor_restrict_unprivileged_userns 2>/dev/null || true)" = "1" ]
}

toml_escape() {
    escaped=$1
    escaped="${escaped//\\/\\\\}"
    escaped="${escaped//\"/\\\"}"
    printf '%s' "$escaped"
}

# Resolve the npm package root when `codex` on PATH is the JS wrapper that npm
# installs at <pkg>/bin/codex.js. That wrapper exec()s a platform-specific
# native binary vendored under <pkg>/node_modules, so the sandbox must be able
# to read the whole package; granting only the wrapper path makes the native
# binary invisible and the probe fails with
#   bwrap: execvp <path>: No such file or directory
# for a file that plainly exists on the host. Prints nothing and returns 1 when
# the executable is not an npm wrapper (e.g. a standalone native build).
codex_npm_package_root() {
    candidate=$1
    case "$candidate" in
        *.js) ;;
        *) return 1 ;;
    esac
    case "$candidate" in
        */bin/*) ;;
        *) return 1 ;;
    esac
    pkg_root=${candidate%/bin/*}
    [ -n "$pkg_root" ] || return 1
    [ -d "$pkg_root" ] && [ ! -L "$pkg_root" ] || return 1
    [ -f "$pkg_root/package.json" ] || return 1
    printf '%s\n' "$pkg_root"
}

permission_profile_filesystem() {
    filesystem="permissions.$PROFILE_NAME.filesystem={\":minimal\"=\"read\",\":workspace_roots\"={\".\"=\"write\"},\"$CODEX_EXECUTABLE_TOML\"=\"read\""
    # The npm entry point is a JS wrapper that exec()s a vendored native
    # binary; granting read on the wrapper alone leaves that binary invisible
    # inside the sandbox. See codex_npm_package_root.
    if [ -n "$CODEX_VENDOR_READ_TOML" ]; then
        filesystem="$filesystem,\"$CODEX_VENDOR_READ_TOML\"=\"read\""
    fi
    # shellcheck disable=SC2088 # Codex expands these config paths, not Bash.
    for denied_path in \
        '~/.aws' \
        '~/.azure' \
        '~/.cargo/credentials' \
        '~/.cargo/credentials.toml' \
        '~/.codex/archived_sessions' \
        '~/.codex/auth.json' \
        '~/.codex/config.toml' \
        '~/.codex/history.jsonl' \
        '~/.codex/sessions' \
        '~/.codex/shell_snapshots' \
        '~/.config/containers' \
        '~/.config/gcloud' \
        '~/.config/gh' \
        '~/.config/pip' \
        '~/.docker' \
        '~/.git-credentials' \
        '~/.gnupg' \
        '~/.gradle' \
        '~/.kube' \
        '~/.m2' \
        '~/.netrc' \
        '~/.npmrc' \
        '~/.pypirc' \
        '~/.ssh' \
        '~/.terraform.d' \
        '~/.vault-token'; do
        filesystem="$filesystem,\"$denied_path\"=\"deny\""
    done

    if [ -n "$ORIGINAL_CODEX_HOME" ]; then
        case "$ORIGINAL_CODEX_HOME" in
            /*) ;;
            *)
                error "$UNAVAILABLE_ERROR: CODEX_HOME must be absolute for the sandbox probe"
                return 1
                ;;
        esac
        default_codex_home="${HOME:-}/.codex"
        if [ "$ORIGINAL_CODEX_HOME" != "$default_codex_home" ]; then
            for sensitive_path in \
                archived_sessions auth.json config.toml history.jsonl sessions shell_snapshots; do
                escaped_path="$(toml_escape "$ORIGINAL_CODEX_HOME/$sensitive_path")"
                filesystem="$filesystem,\"$escaped_path\"=\"deny\""
            done
        fi
    fi
    printf '%s}\n' "$filesystem"
}

probe_codex_permission_profile() {
    filesystem="$(permission_profile_filesystem)" || return 1
    set -- sandbox \
        -C "$PWD" \
        -P "$PROFILE_NAME" \
        -c "default_permissions=\"$PROFILE_NAME\"" \
        -c "$filesystem" \
        -c "permissions.$PROFILE_NAME.network.enabled=true" \
        -c 'shell_environment_policy.inherit="all"' \
        -c 'shell_environment_policy.ignore_default_excludes=false' \
        -c 'shell_environment_policy.exclude=["AWS_*","AZURE_*","CODEX_API_KEY","DOCKER_*","GH_*","GITHUB_*","GOOGLE_*","KUBE*","NPM_*","OPENAI_API_KEY","SSH_*","VAULT_*","*TOKEN*","*SECRET*","*PASSWORD*","*API_KEY*","*CREDENTIAL*"]' \
        -- /bin/true
    if [ "$IGNORE_USER_CONFIG_SUPPORTED" = "1" ]; then
        set -- sandbox --ignore-user-config "${@:2}"
        "$CODEX_EXECUTABLE" "$@"
    else
        CODEX_HOME="$PROBE_CODEX_HOME" "$CODEX_EXECUTABLE" "$@"
    fi
}

is_targeted_bwrap_failure() {
    failure_output=$1
    printf '%s\n' "$failure_output" |
        grep -Fqx -e 'bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted' \
            -e 'bwrap: setting up uid map: Permission denied' \
            -e 'bwrap: setting up uid map: Operation not permitted'
}

# shellcheck disable=SC2016 # This script expands only inside the privileged shell.
PROFILE_TRANSACTION='
set -eu
profile_dir=$1
test_mode=$2
profile_path="$profile_dir/usr.bin.bwrap"
profile_parent=${profile_dir%/*}
candidate=
created=0
expected="abi <abi/4.0>,

include <tunables/global>

/usr/bin/bwrap flags=(unconfined) {
  userns,

  include if exists <local/usr.bin.bwrap>
}"

cleanup() {
    [ -z "$candidate" ] || rm -f -- "$candidate"
}
trap cleanup EXIT INT TERM

matches_expected() {
    [ -f "$1" ] && [ ! -L "$1" ] && [ "$(cat -- "$1")" = "$expected" ]
}

trusted_directory() {
    path=$1
    [ -d "$path" ] && [ ! -L "$path" ] || return 1
    metadata=$(stat -Lc "%u:%g:%a" -- "$path") || return 1
    owner=${metadata%%:*}
    remainder=${metadata#*:}
    group=${remainder%%:*}
    mode=${remainder##*:}
    [ "$owner" = 0 ] && [ "$group" = 0 ] || return 1
    permissions=$((8#$mode))
    [ $((permissions & 0022)) -eq 0 ]
}

rollback_new() {
    installed_id=$1
    current_id=$(stat -Lc "%d:%i" -- "$profile_path" 2>/dev/null || true)
    if [ "$current_id" != "$installed_id" ] || ! matches_expected "$profile_path"; then
        printf "error: codex_sandbox_rollback_refused: installed profile identity changed\n" >&2
        return 1
    fi
    unload_status=0
    apparmor_parser -R "$profile_path" >/dev/null 2>&1 || unload_status=$?
    rm -f -- "$profile_path"
    if [ "$unload_status" -ne 0 ]; then
        printf "error: codex_sandbox_rollback_failed: AppArmor unload failed\n" >&2
        return 1
    fi
}

[ -d "$profile_parent" ] && [ ! -L "$profile_parent" ] &&
    [ -d "$profile_dir" ] && [ ! -L "$profile_dir" ] || {
    printf "error: codex_sandbox_untrusted_profile_dir: symlink or non-directory ancestor\n" >&2
    exit 1
}

if [ "$test_mode" != 1 ]; then
    [ "$profile_dir" = /etc/apparmor.d ] || {
        printf "error: codex_sandbox_untrusted_profile_dir: %s\n" "$profile_dir" >&2
        exit 1
    }
    trusted_directory / && trusted_directory /etc &&
        trusted_directory "$profile_dir" || {
        printf "error: codex_sandbox_untrusted_profile_dir: trusted root-owned ancestors required\n" >&2
        exit 1
    }
fi

if [ -L "$profile_path" ] || { [ -e "$profile_path" ] && [ ! -f "$profile_path" ]; }; then
    printf "error: codex_sandbox_profile_conflict: non-regular profile target %s\n" "$profile_path" >&2
    exit 1
fi

candidate=$(mktemp "$profile_dir/.usr.bin.bwrap.autospec.XXXXXX")
printf "%s\n" "$expected" > "$candidate"
candidate_id=$(stat -Lc "%d:%i" -- "$candidate")
apparmor_parser -Q -K "$candidate"
validated_id=$(stat -Lc "%d:%i" -- "$candidate" 2>/dev/null || true)
if [ "$candidate_id" != "$validated_id" ] || ! matches_expected "$candidate"; then
    printf "error: codex_sandbox_candidate_changed: candidate changed after validation\n" >&2
    exit 1
fi

if [ -e "$profile_path" ]; then
    matches_expected "$profile_path" || {
        printf "error: codex_sandbox_profile_conflict: operator-managed profile at %s\n" "$profile_path" >&2
        exit 1
    }
    if [ "$test_mode" != 1 ]; then
        metadata=$(stat -Lc "%u:%g:%a" -- "$profile_path")
        [ "$metadata" = "0:0:644" ] || {
            printf "error: codex_sandbox_profile_conflict: unsafe profile metadata at %s\n" "$profile_path" >&2
            exit 1
        }
    fi
    existing_id=$(stat -Lc "%d:%i" -- "$profile_path")
    apparmor_parser -Q -K "$profile_path"
    [ "$(stat -Lc "%d:%i" -- "$profile_path" 2>/dev/null || true)" = "$existing_id" ] &&
        matches_expected "$profile_path" || {
        printf "error: codex_sandbox_profile_conflict: profile changed during validation\n" >&2
        exit 1
    }
    if ! apparmor_parser -r "$profile_path"; then
        printf "error: codex_sandbox_host_setup_required: failed to reload existing profile\n" >&2
        exit 1
    fi
    printf "codex_sandbox_profile_state:existing\n"
    exit 0
fi

chmod 0644 "$candidate"
if [ "$test_mode" != 1 ]; then
    chown root:root "$candidate"
fi
installed_id=$(stat -Lc "%d:%i" -- "$candidate")
if ! ln -- "$candidate" "$profile_path"; then
    printf "error: codex_sandbox_profile_conflict: profile appeared during installation\n" >&2
    exit 1
fi
rm -f -- "$candidate"
candidate=
[ "$(stat -Lc "%d:%i" -- "$profile_path")" = "$installed_id" ] &&
    matches_expected "$profile_path" || {
    printf "error: codex_sandbox_candidate_changed: installed profile identity mismatch\n" >&2
    rollback_new "$installed_id" || true
    exit 1
}
if ! apparmor_parser -r "$profile_path"; then
    printf "error: codex_sandbox_host_setup_required: failed to reload new profile\n" >&2
    rollback_new "$installed_id" || true
    exit 1
fi
printf "codex_sandbox_profile_state:new:%s\n" "$installed_id"
'

# shellcheck disable=SC2016 # This script expands only inside the privileged shell.
ROLLBACK_TRANSACTION='
set -eu
profile_dir=$1
test_mode=$2
expected_id=$3
profile_path="$profile_dir/usr.bin.bwrap"
profile_parent=${profile_dir%/*}
expected="abi <abi/4.0>,

include <tunables/global>

/usr/bin/bwrap flags=(unconfined) {
  userns,

  include if exists <local/usr.bin.bwrap>
}"
[ -d "$profile_parent" ] && [ ! -L "$profile_parent" ] &&
    [ -d "$profile_dir" ] && [ ! -L "$profile_dir" ] || {
    printf "error: codex_sandbox_rollback_refused: untrusted profile ancestor\n" >&2
    exit 1
}
if [ "$test_mode" != 1 ] && [ "$profile_dir" != /etc/apparmor.d ]; then
    printf "error: codex_sandbox_rollback_refused: untrusted profile directory\n" >&2
    exit 1
fi
current_id=$(stat -Lc "%d:%i" -- "$profile_path" 2>/dev/null || true)
if [ "$current_id" != "$expected_id" ] || [ -L "$profile_path" ] ||
    [ ! -f "$profile_path" ] || [ "$(cat -- "$profile_path")" != "$expected" ]; then
    printf "error: codex_sandbox_rollback_refused: profile identity changed\n" >&2
    exit 1
fi
unload_status=0
apparmor_parser -R "$profile_path" >/dev/null 2>&1 || unload_status=$?
rm -f -- "$profile_path"
if [ "$unload_status" -ne 0 ]; then
    printf "error: codex_sandbox_rollback_failed: AppArmor unload failed\n" >&2
    exit 1
fi
'

run_boundary() {
    boundary_script=$1
    shift
    if [ "$TEST_MODE" = "1" ]; then
        /bin/bash -c "$boundary_script" autospec-codex-sandbox "$PROFILE_DIR" 1 "$@"
    elif [ "$(/usr/bin/id -u 2>/dev/null || printf '1')" = "0" ]; then
        run_clean_shell "$boundary_script" /etc/apparmor.d 0 "$@"
    elif [ -x /usr/bin/sudo ]; then
        /usr/bin/sudo /usr/bin/env -i \
            "PATH=$CLEAN_BOUNDARY_PATH" \
            /bin/bash --noprofile --norc -p -c "$boundary_script" \
            autospec-codex-sandbox /etc/apparmor.d 0 "$@"
    else
        error "$HOST_ERROR: root privileges are required, but sudo is unavailable"
        return 1
    fi
}

if [ "$(uname -s 2>/dev/null || true)" != "Linux" ]; then
    printf 'ensure_codex_sandbox: not applicable outside Linux\n'
    exit 0
fi

if ! command -v codex >/dev/null 2>&1; then
    printf 'ensure_codex_sandbox: codex CLI is not installed; skipping probe\n'
    exit 0
fi

CODEX_EXECUTABLE="$(readlink -f "$(command -v codex)" 2>/dev/null || true)"
if [ -z "$CODEX_EXECUTABLE" ] || [ ! -f "$CODEX_EXECUTABLE" ]; then
    error "$UNAVAILABLE_ERROR: cannot resolve the Codex executable from PATH"
    exit 1
fi
CODEX_EXECUTABLE_TOML="$(toml_escape "$CODEX_EXECUTABLE")"
codex_pkg_root="$(codex_npm_package_root "$CODEX_EXECUTABLE" || true)"
if [ -n "$codex_pkg_root" ]; then
    CODEX_VENDOR_READ_TOML="$(toml_escape "$codex_pkg_root")"
fi
# `codex exec` supports --ignore-user-config on 0.144.4, but `codex sandbox`
# does not. Probe the exact subcommand capability instead of assuming parity.
if "$CODEX_EXECUTABLE" sandbox --help 2>/dev/null |
    grep -q -- '--ignore-user-config'; then
    IGNORE_USER_CONFIG_SUPPORTED=1
else
    PROBE_CODEX_HOME="$(mktemp -d)"
fi

probe_output=""
if probe_output="$(probe_codex_permission_profile 2>&1)"; then
    printf 'ensure_codex_sandbox: Codex permission-profile probe passed\n'
    exit 0
fi

if ! is_targeted_bwrap_failure "$probe_output"; then
    error "$UNAVAILABLE_ERROR: Codex permission-profile probe failed: $probe_output"
    exit 1
fi

if ! is_ubuntu || ! apparmor_restricts_userns; then
    error "$HOST_ERROR: targeted bwrap failure was observed without the Ubuntu AppArmor userns restriction"
    exit 1
fi

if [ "${AUTOSPEC_SKIP_SYSTEM_TOOLS:-0}" = "1" ]; then
    error "$HOST_ERROR: AUTOSPEC_SKIP_SYSTEM_TOOLS=1 prevents the required AppArmor profile write"
    exit 1
fi

if ! command -v apparmor_parser >/dev/null 2>&1; then
    error "$HOST_ERROR: apparmor_parser is required to validate and reload the profile"
    exit 1
fi

transaction_output=""
if ! transaction_output="$(run_boundary "$PROFILE_TRANSACTION" 2>&1)"; then
    printf '%s\n' "$transaction_output" >&2
    exit 1
fi
printf '%s\n' "$transaction_output"

profile_state="$(printf '%s\n' "$transaction_output" |
    sed -n 's/^codex_sandbox_profile_state://p' | tail -1)"

if ! probe_output="$(probe_codex_permission_profile 2>&1)"; then
    if case "$profile_state" in new:*) true ;; *) false ;; esac; then
        installed_id="${profile_state#new:}"
        if ! run_boundary "$ROLLBACK_TRANSACTION" "$installed_id"; then
            error "codex_sandbox_rollback_failed: manual recovery is required for $PROFILE_PATH"
        fi
    fi
    error "$HOST_ERROR: AppArmor profile was installed, but the Codex probe still fails: $probe_output"
    exit 1
fi

printf 'ensure_codex_sandbox: installed and verified %s\n' "$PROFILE_PATH"

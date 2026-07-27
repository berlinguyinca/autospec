#!/usr/bin/env bash
# Repair the Ubuntu AppArmor restriction that prevents Codex permission
# profiles from creating Bubblewrap user namespaces.

set -eu

PROFILE_NAME="autospec-network-executor"
PROFILE_ERROR="codex_sandbox_profile_conflict"
HOST_ERROR="codex_sandbox_host_setup_required"
UNAVAILABLE_ERROR="codex_sandbox_unavailable"
SYSTEM_ROOT="${AUTOSPEC_CODEX_SANDBOX_ROOT:-}"
CODEX_EXECUTABLE=""
CODEX_EXECUTABLE_TOML=""

case "$SYSTEM_ROOT" in
    "") ;;
    /*) SYSTEM_ROOT="${SYSTEM_ROOT%/}" ;;
    *)
        printf 'error: %s: AUTOSPEC_CODEX_SANDBOX_ROOT must be absolute\n' \
            "$UNAVAILABLE_ERROR" >&2
        exit 1
        ;;
esac

OS_RELEASE="$SYSTEM_ROOT/etc/os-release"
PROFILE_PATH="$SYSTEM_ROOT/etc/apparmor.d/usr.bin.bwrap"
TEMP_PROFILE=""

cleanup() {
    [ -z "$TEMP_PROFILE" ] || rm -f "$TEMP_PROFILE"
}
trap cleanup EXIT INT TERM

error() {
    printf 'error: %s\n' "$*" >&2
}

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

probe_codex_permission_profile() {
    "$CODEX_EXECUTABLE" sandbox \
        -C "$PWD" \
        -P "$PROFILE_NAME" \
        -c "default_permissions=\"$PROFILE_NAME\"" \
        -c "permissions.$PROFILE_NAME.filesystem={\":minimal\"=\"read\",\":workspace_roots\"={\".\"=\"write\"},\"$CODEX_EXECUTABLE_TOML\"=\"read\"}" \
        -c "permissions.$PROFILE_NAME.network.enabled=true" \
        -- /bin/true
}

is_targeted_bwrap_failure() {
    failure_output=$1
    printf '%s\n' "$failure_output" |
        grep -Fqx -e 'bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted' \
            -e 'bwrap: setting up uid map: Permission denied' \
            -e 'bwrap: setting up uid map: Operation not permitted'
}

run_privileged() {
    if [ "$(id -u 2>/dev/null || printf '1')" = "0" ]; then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo "$@"
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
CODEX_EXECUTABLE_TOML="${CODEX_EXECUTABLE//\\/\\\\}"
CODEX_EXECUTABLE_TOML="${CODEX_EXECUTABLE_TOML//\"/\\\"}"

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

TEMP_PROFILE="$(mktemp)"
cat > "$TEMP_PROFILE" <<'PROFILE'
abi <abi/4.0>,

include <tunables/global>

/usr/bin/bwrap flags=(unconfined) {
  userns,

  include if exists <local/usr.bin.bwrap>
}
PROFILE

if [ -L "$PROFILE_PATH" ] ||
    { [ -e "$PROFILE_PATH" ] && ! cmp -s "$TEMP_PROFILE" "$PROFILE_PATH"; }; then
    error "$PROFILE_ERROR: $PROFILE_PATH already contains an operator-managed profile; refusing to overwrite it"
    exit 1
fi

if ! command -v apparmor_parser >/dev/null 2>&1; then
    error "$HOST_ERROR: apparmor_parser is required to validate and reload the profile"
    exit 1
fi

if ! apparmor_parser -Q -K "$TEMP_PROFILE"; then
    error "$HOST_ERROR: apparmor_parser rejected the targeted profile"
    exit 1
fi

if ! run_privileged install -o root -g root -m 0644 "$TEMP_PROFILE" "$PROFILE_PATH"; then
    error "$HOST_ERROR: failed to install $PROFILE_PATH"
    exit 1
fi

if ! run_privileged apparmor_parser -r "$PROFILE_PATH"; then
    error "$HOST_ERROR: failed to reload $PROFILE_PATH"
    exit 1
fi

if ! probe_output="$(probe_codex_permission_profile 2>&1)"; then
    error "$HOST_ERROR: AppArmor profile was installed, but the Codex probe still fails: $probe_output"
    exit 1
fi

printf 'ensure_codex_sandbox: installed and verified %s\n' "$PROFILE_PATH"

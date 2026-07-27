#!/usr/bin/env bash
set -eu

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HELPER="$ROOT/scripts/ensure-codex-sandbox.sh"
TMP_DIR="$(mktemp -d)"
FAKE_BIN="$TMP_DIR/bin"
FAILURES=0

cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    FAILURES=$((FAILURES + 1))
}

mkdir -p "$FAKE_BIN"

cat > "$FAKE_BIN/codex" <<'SHIM'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$CODEX_LOG"
case "${CODEX_MODE:-blocked}" in
    healthy)
        exit 0
        ;;
    blocked)
        if [ -f "$REPAIRED_MARKER" ]; then
            exit 0
        fi
        printf '%s\n' 'bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted' >&2
        exit 1
        ;;
    uid-map)
        if [ -f "$REPAIRED_MARKER" ]; then
            exit 0
        fi
        printf '%s\n' 'bwrap: setting up uid map: Operation not permitted' >&2
        exit 1
        ;;
    unrelated)
        printf '%s\n' 'codex configuration is invalid' >&2
        exit 1
        ;;
    package)
        case "$*" in
            *"\"$CODEX_REAL_PATH\"=\"read\""*) exit 0 ;;
        esac
        printf 'bwrap: execvp %s: No such file or directory\n' "$CODEX_REAL_PATH" >&2
        exit 1
        ;;
esac
SHIM

mkdir -p "$TMP_DIR/codex-package/bin"
mv "$FAKE_BIN/codex" "$TMP_DIR/codex-package/bin/codex"
ln -s "$TMP_DIR/codex-package/bin/codex" "$FAKE_BIN/codex"

cat > "$FAKE_BIN/sysctl" <<'SHIM'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$SYSCTL_LOG"
if [ "$*" = "-n kernel.apparmor_restrict_unprivileged_userns" ]; then
    printf '%s\n' "${APPARMOR_RESTRICTED:-1}"
    exit 0
fi
exit 1
SHIM

cat > "$FAKE_BIN/apparmor_parser" <<'SHIM'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$PARSER_LOG"
case "${1:-}" in
    -Q)
        [ "${2:-}" = "-K" ]
        grep -Fqx 'abi <abi/4.0>,' "$3"
        grep -Fqx '/usr/bin/bwrap flags=(unconfined) {' "$3"
        grep -Fqx '  userns,' "$3"
        grep -Fqx '  include if exists <local/usr.bin.bwrap>' "$3"
        ;;
    -r)
        touch "$REPAIRED_MARKER"
        ;;
    *)
        exit 2
        ;;
esac
SHIM

cat > "$FAKE_BIN/id" <<'SHIM'
#!/usr/bin/env bash
if [ "${1:-}" = "-u" ]; then
    printf '%s\n' '1000'
    exit 0
fi
exec /usr/bin/id "$@"
SHIM

cat > "$FAKE_BIN/sudo" <<'SHIM'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$PRIVILEGED_LOG"
exec "$@"
SHIM

cat > "$FAKE_BIN/install" <<'SHIM'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$INSTALL_LOG"
owner=
group=
mode=
while [ "$#" -gt 2 ]; do
    case "$1" in
        -o) owner=$2; shift 2 ;;
        -g) group=$2; shift 2 ;;
        -m) mode=$2; shift 2 ;;
        *) exit 2 ;;
    esac
done
[ "$owner" = root ]
[ "$group" = root ]
[ "$mode" = 0644 ]
mkdir -p "$(dirname "$2")"
cp "$1" "$2"
SHIM

chmod +x "$FAKE_BIN"/*

run_helper() {
    case_name=$1
    shift
    case_dir="$TMP_DIR/$case_name"
    system_root="$case_dir/root"
    mkdir -p "$system_root/etc/apparmor.d"
    printf 'ID=%s\n' "${CASE_OS_ID:-ubuntu}" > "$system_root/etc/os-release"
    : > "$case_dir/codex.log"
    : > "$case_dir/sysctl.log"
    : > "$case_dir/parser.log"
    : > "$case_dir/privileged.log"
    : > "$case_dir/install.log"

    set +e
    CASE_OUTPUT=$(env \
        PATH="$FAKE_BIN:$PATH" \
        AUTOSPEC_CODEX_SANDBOX_ROOT="$system_root" \
        CODEX_LOG="$case_dir/codex.log" \
        SYSCTL_LOG="$case_dir/sysctl.log" \
        PARSER_LOG="$case_dir/parser.log" \
        PRIVILEGED_LOG="$case_dir/privileged.log" \
        INSTALL_LOG="$case_dir/install.log" \
        REPAIRED_MARKER="$case_dir/repaired" \
        "$@" \
        bash "$HELPER" 2>&1)
    CASE_STATUS=$?
    set -e
}

if [ ! -f "$HELPER" ]; then
    fail "Codex sandbox dependency helper is absent"
else
    run_helper healthy CODEX_MODE=healthy
    if [ "$CASE_STATUS" -ne 0 ]; then
        fail "a healthy Codex permission profile was rejected: $CASE_OUTPUT"
    fi
    if [ -s "$TMP_DIR/healthy/privileged.log" ]; then
        fail "healthy probe performed privileged writes"
    fi
    if ! grep -Fq -- '-P autospec-network-executor' "$TMP_DIR/healthy/codex.log"; then
        fail "probe did not exercise the named Codex permission profile"
    fi
    if ! grep -Fq 'permissions.autospec-network-executor.network.enabled=true' "$TMP_DIR/healthy/codex.log"; then
        fail "probe did not exercise network-enabled permission-profile behavior"
    fi

    run_helper package-tree \
        CODEX_MODE=package \
        CODEX_REAL_PATH="$TMP_DIR/codex-package/bin/codex"
    if [ "$CASE_STATUS" -ne 0 ]; then
        fail "probe could not execute Codex from its resolved package path: $CASE_OUTPUT"
    fi
    if [ -s "$TMP_DIR/package-tree/privileged.log" ]; then
        fail "Codex package accessibility failure triggered privileged writes"
    fi

    run_helper repaired CODEX_MODE=blocked
    if [ "$CASE_STATUS" -ne 0 ]; then
        fail "restricted Ubuntu host was not repaired: $CASE_OUTPUT"
    fi
    repaired_profile="$TMP_DIR/repaired/root/etc/apparmor.d/usr.bin.bwrap"
    if [ ! -f "$repaired_profile" ]; then
        fail "repair did not install the targeted bwrap profile"
    fi
    if ! grep -Fqx '/usr/bin/bwrap flags=(unconfined) {' "$repaired_profile" ||
        ! grep -Fqx '  userns,' "$repaired_profile" ||
        ! grep -Fqx '  include if exists <local/usr.bin.bwrap>' "$repaired_profile"; then
        fail "installed profile does not contain the targeted userns allowance"
    fi
    if ! grep -Eq '^-o root -g root -m 0644 .+ /.+/etc/apparmor.d/usr.bin.bwrap$' \
        "$TMP_DIR/repaired/install.log"; then
        fail "profile was not installed with root:root mode 0644"
    fi
    if ! grep -Eq '^-Q -K .+' "$TMP_DIR/repaired/parser.log"; then
        fail "profile was not validated with apparmor_parser -Q without cache writes"
    fi
    if ! grep -Fqx "apparmor_parser -r $repaired_profile" \
        "$TMP_DIR/repaired/privileged.log"; then
        fail "installed profile was not reloaded through sudo"
    fi
    if [ "$(wc -l < "$TMP_DIR/repaired/codex.log")" -ne 2 ]; then
        fail "repair did not re-probe Codex exactly once"
    fi

    privileged_before="$(wc -l < "$TMP_DIR/repaired/privileged.log")"
    set +e
    second_output=$(env \
        PATH="$FAKE_BIN:$PATH" \
        AUTOSPEC_CODEX_SANDBOX_ROOT="$TMP_DIR/repaired/root" \
        CODEX_LOG="$TMP_DIR/repaired/codex.log" \
        SYSCTL_LOG="$TMP_DIR/repaired/sysctl.log" \
        PARSER_LOG="$TMP_DIR/repaired/parser.log" \
        PRIVILEGED_LOG="$TMP_DIR/repaired/privileged.log" \
        INSTALL_LOG="$TMP_DIR/repaired/install.log" \
        REPAIRED_MARKER="$TMP_DIR/repaired/repaired" \
        CODEX_MODE=blocked \
        bash "$HELPER" 2>&1)
    second_status=$?
    set -e
    privileged_after="$(wc -l < "$TMP_DIR/repaired/privileged.log")"
    if [ "$second_status" -ne 0 ]; then
        fail "healthy repeat failed: $second_output"
    fi
    if [ "$privileged_before" != "$privileged_after" ]; then
        fail "idempotent repeat added privileged writes"
    fi

    run_helper opt-out CODEX_MODE=uid-map AUTOSPEC_SKIP_SYSTEM_TOOLS=1
    if [ "$CASE_STATUS" -eq 0 ]; then
        fail "system-tool opt-out silently accepted a blocked Codex sandbox"
    fi
    case "$CASE_OUTPUT" in
        *"codex_sandbox_host_setup_required"*"AUTOSPEC_SKIP_SYSTEM_TOOLS=1"*) ;;
        *) fail "opt-out did not report the typed host setup blocker: $CASE_OUTPUT" ;;
    esac
    if [ -s "$TMP_DIR/opt-out/privileged.log" ] ||
        [ -e "$TMP_DIR/opt-out/root/etc/apparmor.d/usr.bin.bwrap" ]; then
        fail "system-tool opt-out performed a privileged profile write"
    fi

    conflict_profile="$TMP_DIR/conflict/root/etc/apparmor.d/usr.bin.bwrap"
    mkdir -p "$(dirname "$conflict_profile")"
    printf '%s\n' 'operator-managed profile' > "$conflict_profile"
    run_helper conflict CODEX_MODE=blocked
    # run_helper creates the root after the conflicting fixture, but preserves files.
    if [ "$CASE_STATUS" -eq 0 ]; then
        fail "different existing bwrap profile was overwritten"
    fi
    case "$CASE_OUTPUT" in
        *"codex_sandbox_profile_conflict"*) ;;
        *) fail "profile conflict did not emit its typed error: $CASE_OUTPUT" ;;
    esac
    if [ "$(cat "$conflict_profile")" != 'operator-managed profile' ]; then
        fail "different existing bwrap profile changed"
    fi
    if [ -s "$TMP_DIR/conflict/privileged.log" ]; then
        fail "profile conflict attempted a privileged write"
    fi

    run_helper unrelated CODEX_MODE=unrelated
    if [ "$CASE_STATUS" -eq 0 ]; then
        fail "unrelated Codex probe failure was accepted"
    fi
    if [ -s "$TMP_DIR/unrelated/privileged.log" ]; then
        fail "unrelated Codex probe failure triggered AppArmor writes"
    fi

    CASE_OS_ID=debian run_helper non-ubuntu CODEX_MODE=blocked
    if [ "$CASE_STATUS" -eq 0 ] || [ -s "$TMP_DIR/non-ubuntu/privileged.log" ]; then
        fail "non-Ubuntu Linux host triggered the Ubuntu-specific repair"
    fi

    run_helper unrestricted-kernel CODEX_MODE=blocked APPARMOR_RESTRICTED=0
    if [ "$CASE_STATUS" -eq 0 ] || [ -s "$TMP_DIR/unrestricted-kernel/privileged.log" ]; then
        fail "host without the AppArmor userns restriction triggered the repair"
    fi
fi

dry_output=$(AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    bash "$ROOT/install.sh" --dry-run --skill all --harness all 2>&1 || true)
sandbox_line=$(printf '%s\n' "$dry_output" | grep -n 'ensure_codex_sandbox_dependency' | head -1 | cut -d: -f1 || true)
first_skill_line=$(printf '%s\n' "$dry_output" | grep -n '^==> ' | head -1 | cut -d: -f1 || true)
if [ -z "$sandbox_line" ] || [ -z "$first_skill_line" ] || [ "$sandbox_line" -ge "$first_skill_line" ]; then
    fail "install.sh did not run the Codex sandbox helper before autonomous skill installation"
fi

if [ "$FAILURES" -ne 0 ]; then
    printf 'FAIL: %s Codex sandbox dependency assertion(s)\n' "$FAILURES" >&2
    exit 1
fi

printf 'PASS: Codex sandbox dependency repair is targeted, safe, and idempotent\n'

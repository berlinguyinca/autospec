#!/usr/bin/env bash
set -eu

if [ "$(uname -s)" != Linux ]; then
    printf 'SKIP: Codex AppArmor sandbox dependency applies only on Linux\n'
    exit 0
fi

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
if [ "$*" = "sandbox --help" ]; then
    if [ "${CODEX_SUPPORTS_IGNORE_USER_CONFIG:-1}" = "1" ]; then
        printf '%s\n' '      --ignore-user-config'
    fi
    exit 0
fi
printf 'CODEX_HOME=%s %s\n' "${CODEX_HOME:-}" "$*" >> "$CODEX_LOG"
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
    post-fail)
        if [ -f "$REPAIRED_MARKER" ]; then
            printf '%s\n' 'codex post-install probe failed' >&2
            exit 1
        fi
        printf '%s\n' 'bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted' >&2
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
        if [ "${APPARMOR_MUTATE_AFTER_VALIDATE:-0}" = "1" ]; then
            printf '%s\n' '/usr/bin/bwrap flags=(unconfined) { /** rw, }' > "$3"
        fi
        ;;
    -r)
        if [ "${APPARMOR_RELOAD_FAIL:-0}" = "1" ]; then
            exit 1
        fi
        touch "$REPAIRED_MARKER"
        ;;
    -R)
        rm -f "$REPAIRED_MARKER"
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

chmod +x "$FAKE_BIN"/*

BOUNDARY_ATTACK_MARKER="$TMP_DIR/boundary-attack-ran"
cat > "$FAKE_BIN/autospec-boundary-attack" <<SHIM
#!/usr/bin/env bash
touch "$BOUNDARY_ATTACK_MARKER"
SHIM
chmod +x "$FAKE_BIN/autospec-boundary-attack"
printf '%s\n' 'export AUTOSPEC_BASH_ENV_LOADED=1' > "$TMP_DIR/attacker-bash-env"

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
    mkdir -p "$case_dir/codex-home"

    set +e
    CASE_OUTPUT=$(env \
        PATH="$FAKE_BIN:$PATH" \
        AUTOSPEC_CODEX_SANDBOX_ROOT="$system_root" \
        AUTOSPEC_CODEX_SANDBOX_TEST_MODE=1 \
        CODEX_LOG="$case_dir/codex.log" \
        SYSCTL_LOG="$case_dir/sysctl.log" \
        PARSER_LOG="$case_dir/parser.log" \
        PRIVILEGED_LOG="$case_dir/privileged.log" \
        REPAIRED_MARKER="$case_dir/repaired" \
        CODEX_HOME="$case_dir/codex-home" \
        "$@" \
        bash "$HELPER" 2>&1)
    CASE_STATUS=$?
    set -e
}

if [ ! -f "$HELPER" ]; then
    fail "Codex sandbox dependency helper is absent"
else
    clean_root="$TMP_DIR/clean-boundary/root"
    mkdir -p "$clean_root/etc/apparmor.d"
    set +e
    clean_output=$(env \
        PATH="$FAKE_BIN:$PATH" \
        BASH_ENV="$TMP_DIR/attacker-bash-env" \
        AUTOSPEC_CODEX_SANDBOX_ROOT="$clean_root" \
        AUTOSPEC_CODEX_SANDBOX_TEST_MODE=1 \
        bash "$HELPER" --test-clean-boundary 2>&1)
    clean_status=$?
    set -e
    if [ "$clean_status" -ne 0 ] ||
        [ "$clean_output" != "codex_sandbox_clean_boundary:verified" ]; then
        fail "production-equivalent clean boundary was not verified: $clean_output"
    fi
    if [ -e "$BOUNDARY_ATTACK_MARKER" ]; then
        fail "attacker PATH executable ran inside the clean boundary"
    fi

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
    for expected_arg in \
        '--ignore-user-config' \
        'shell_environment_policy.inherit="all"' \
        'shell_environment_policy.ignore_default_excludes=false' \
        "\"$TMP_DIR/healthy/codex-home/auth.json\"=\"deny\"" \
        'shell_environment_policy.exclude=["AWS_*","AZURE_*","CODEX_API_KEY","DOCKER_*","GH_*","GITHUB_*","GOOGLE_*","KUBE*","NPM_*","OPENAI_API_KEY","SSH_*","VAULT_*","*TOKEN*","*SECRET*","*PASSWORD*","*API_KEY*","*CREDENTIAL*"]'; do
        if ! grep -Fq -- "$expected_arg" "$TMP_DIR/healthy/codex.log"; then
            fail "probe omitted canonical executor policy argument: $expected_arg"
        fi
    done
    # shellcheck disable=SC2088 # Assert literal Codex config paths.
    for denied_path in \
        '~/.aws' '~/.azure' '~/.cargo/credentials' '~/.cargo/credentials.toml' \
        '~/.codex/archived_sessions' '~/.codex/auth.json' '~/.codex/config.toml' \
        '~/.codex/history.jsonl' '~/.codex/sessions' '~/.codex/shell_snapshots' \
        '~/.config/containers' '~/.config/gcloud' '~/.config/gh' '~/.config/pip' \
        '~/.docker' '~/.git-credentials' '~/.gnupg' '~/.gradle' '~/.kube' \
        '~/.m2' '~/.netrc' '~/.npmrc' '~/.pypirc' '~/.ssh' '~/.terraform.d' \
        '~/.vault-token'; do
        if ! grep -Fq -- "\"$denied_path\"=\"deny\"" "$TMP_DIR/healthy/codex.log"; then
            fail "probe omitted canonical sensitive-path deny: $denied_path"
        fi
    done

    run_helper legacy-codex \
        CODEX_MODE=healthy \
        CODEX_SUPPORTS_IGNORE_USER_CONFIG=0
    if [ "$CASE_STATUS" -ne 0 ]; then
        fail "Codex without sandbox --ignore-user-config support was rejected: $CASE_OUTPUT"
    fi
    if grep -Fq -- '--ignore-user-config' "$TMP_DIR/legacy-codex/codex.log" ||
        ! grep -Eq '^CODEX_HOME=/tmp/.+ sandbox ' "$TMP_DIR/legacy-codex/codex.log"; then
        fail "legacy Codex probe did not use an isolated temporary CODEX_HOME"
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
    if [ "$(stat -c '%a' "$repaired_profile")" != "644" ]; then
        fail "profile was not installed with mode 0644"
    fi
    if ! grep -Eq '^-Q -K .+' "$TMP_DIR/repaired/parser.log"; then
        fail "profile was not validated with apparmor_parser -Q without cache writes"
    fi
    if ! grep -Fqx -- "-r $repaired_profile" "$TMP_DIR/repaired/parser.log"; then
        fail "installed profile was not reloaded inside the profile transaction"
    fi
    if [ -s "$TMP_DIR/repaired/privileged.log" ]; then
        fail "test-root injection reached the production sudo path"
    fi
    if [ "$(wc -l < "$TMP_DIR/repaired/codex.log")" -ne 2 ]; then
        fail "repair did not re-probe Codex exactly once"
    fi

    privileged_before="$(wc -l < "$TMP_DIR/repaired/privileged.log")"
    set +e
    second_output=$(env \
        PATH="$FAKE_BIN:$PATH" \
        AUTOSPEC_CODEX_SANDBOX_ROOT="$TMP_DIR/repaired/root" \
        AUTOSPEC_CODEX_SANDBOX_TEST_MODE=1 \
        CODEX_LOG="$TMP_DIR/repaired/codex.log" \
        SYSCTL_LOG="$TMP_DIR/repaired/sysctl.log" \
        PARSER_LOG="$TMP_DIR/repaired/parser.log" \
        PRIVILEGED_LOG="$TMP_DIR/repaired/privileged.log" \
        REPAIRED_MARKER="$TMP_DIR/repaired/repaired" \
        CODEX_HOME="$TMP_DIR/repaired/codex-home" \
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

    run_helper tampered-candidate \
        CODEX_MODE=blocked \
        APPARMOR_MUTATE_AFTER_VALIDATE=1
    if [ "$CASE_STATUS" -eq 0 ]; then
        fail "candidate mutation after validation was accepted"
    fi
    if [ -e "$TMP_DIR/tampered-candidate/root/etc/apparmor.d/usr.bin.bwrap" ]; then
        fail "mutated candidate reached the installed profile path"
    fi
    if grep -q '^-r ' "$TMP_DIR/tampered-candidate/parser.log"; then
        fail "mutated candidate reached AppArmor reload"
    fi

    run_helper reload-failure CODEX_MODE=blocked APPARMOR_RELOAD_FAIL=1
    if [ "$CASE_STATUS" -eq 0 ] ||
        [ -e "$TMP_DIR/reload-failure/root/etc/apparmor.d/usr.bin.bwrap" ]; then
        fail "reload failure left a newly installed profile behind"
    fi
    if ! grep -q '^-R ' "$TMP_DIR/reload-failure/parser.log"; then
        fail "reload failure did not attempt to unload the newly installed profile"
    fi

    run_helper post-probe-failure CODEX_MODE=post-fail
    if [ "$CASE_STATUS" -eq 0 ] ||
        [ -e "$TMP_DIR/post-probe-failure/root/etc/apparmor.d/usr.bin.bwrap" ]; then
        fail "post-install probe failure left a newly installed profile behind"
    fi
    if ! grep -q '^-R ' "$TMP_DIR/post-probe-failure/parser.log"; then
        fail "post-install probe failure did not unload the new profile"
    fi

    preexisting_profile="$TMP_DIR/preexisting-identical/root/etc/apparmor.d/usr.bin.bwrap"
    mkdir -p "$(dirname "$preexisting_profile")"
    cp "$repaired_profile" "$preexisting_profile"
    run_helper preexisting-identical CODEX_MODE=blocked APPARMOR_RELOAD_FAIL=1
    if [ "$CASE_STATUS" -eq 0 ] || [ ! -f "$preexisting_profile" ]; then
        fail "reload failure did not preserve a pre-existing identical profile"
    fi
    if grep -q '^-R ' "$TMP_DIR/preexisting-identical/parser.log"; then
        fail "reload failure unloaded a pre-existing identical profile"
    fi

    symlink_target="$TMP_DIR/operator-profile"
    printf '%s\n' 'operator policy' > "$symlink_target"
    symlink_profile="$TMP_DIR/symlink-conflict/root/etc/apparmor.d/usr.bin.bwrap"
    mkdir -p "$(dirname "$symlink_profile")"
    ln -s "$symlink_target" "$symlink_profile"
    run_helper symlink-conflict CODEX_MODE=blocked
    if [ "$CASE_STATUS" -eq 0 ] || [ "$(cat "$symlink_target")" != "operator policy" ]; then
        fail "symlink profile target was not rejected and preserved"
    fi
    case "$CASE_OUTPUT" in
        *"codex_sandbox_profile_conflict"*) ;;
        *) fail "symlink profile conflict did not emit its typed error: $CASE_OUTPUT" ;;
    esac

    mkdir -p "$TMP_DIR/parent-symlink/root" "$TMP_DIR/parent-symlink/redirect/apparmor.d"
    ln -s "$TMP_DIR/parent-symlink/redirect" "$TMP_DIR/parent-symlink/root/etc"
    run_helper parent-symlink CODEX_MODE=blocked
    if [ "$CASE_STATUS" -eq 0 ] ||
        [ -e "$TMP_DIR/parent-symlink/redirect/apparmor.d/usr.bin.bwrap" ]; then
        fail "symlinked AppArmor parent reached profile installation"
    fi
    case "$CASE_OUTPUT" in
        *"codex_sandbox_untrusted_profile_dir"*) ;;
        *) fail "symlinked AppArmor parent did not emit its typed refusal: $CASE_OUTPUT" ;;
    esac

    unsafe_root="$TMP_DIR/unsafe-override/root"
    mkdir -p "$unsafe_root/etc/apparmor.d"
    printf '%s\n' 'ID=ubuntu' > "$unsafe_root/etc/os-release"
    : > "$TMP_DIR/unsafe-override.privileged.log"
    set +e
    unsafe_output=$(env \
        PATH="$FAKE_BIN:$PATH" \
        AUTOSPEC_CODEX_SANDBOX_ROOT="$unsafe_root" \
        CODEX_LOG="$TMP_DIR/unsafe-override.codex.log" \
        SYSCTL_LOG="$TMP_DIR/unsafe-override.sysctl.log" \
        PARSER_LOG="$TMP_DIR/unsafe-override.parser.log" \
        PRIVILEGED_LOG="$TMP_DIR/unsafe-override.privileged.log" \
        REPAIRED_MARKER="$TMP_DIR/unsafe-override.repaired" \
        CODEX_HOME="$TMP_DIR/unsafe-override.codex-home" \
        CODEX_MODE=blocked \
        bash "$HELPER" 2>&1)
    unsafe_status=$?
    set -e
    if [ "$unsafe_status" -eq 0 ]; then
        fail "arbitrary sandbox root was accepted outside explicit test mode"
    fi
    case "$unsafe_output" in
        *"codex_sandbox_test_root_refused"*) ;;
        *) fail "unsafe root override did not emit its typed refusal: $unsafe_output" ;;
    esac
    if [ -s "$TMP_DIR/unsafe-override.privileged.log" ]; then
        fail "unsafe root override reached sudo"
    fi

    # Regression: npm installs `codex` as a JS wrapper at <pkg>/bin/codex.js
    # that locates its platform-specific native binary through Node resolution
    # and exec()s it. Granting the sandbox read access to the wrapper alone
    # leaves that binary invisible inside bwrap, and the probe dies with
    # `execvp: <path>: No such file or directory` for a file that exists.
    #
    # The platform package sits in a different place per install layout, so
    # every supported layout is exercised and the assertion is CONTAINMENT of
    # the real vendored binary, not equality with one hard-coded directory.
    run_layout_case() {
        layout_name=$1
        layout_wrapper=$2
        layout_vendor_bin=$3

        layout_dir="$TMP_DIR/layout-$layout_name"
        layout_path_bin="$layout_dir/pathbin"
        layout_sysroot="$layout_dir/root"
        mkdir -p "$layout_path_bin" "$layout_sysroot/etc/apparmor.d" \
            "$layout_dir/codex-home" \
            "$(dirname "$layout_wrapper")" "$(dirname "$layout_vendor_bin")"
        printf '%s\n' 'ID=ubuntu' > "$layout_sysroot/etc/os-release"
        cp "$FAKE_BIN/codex" "$layout_wrapper"
        chmod +x "$layout_wrapper"
        printf '%s\n' 'native' > "$layout_vendor_bin"
        chmod +x "$layout_vendor_bin"
        ln -sf "$layout_wrapper" "$layout_path_bin/codex"
        : > "$layout_dir/codex.log"

        set +e
        layout_output=$(env \
            PATH="$layout_path_bin:$FAKE_BIN:$PATH" \
            AUTOSPEC_CODEX_SANDBOX_ROOT="$layout_sysroot" \
            AUTOSPEC_CODEX_SANDBOX_TEST_MODE=1 \
            CODEX_LOG="$layout_dir/codex.log" \
            SYSCTL_LOG="$layout_dir/sysctl.log" \
            PARSER_LOG="$layout_dir/parser.log" \
            PRIVILEGED_LOG="$layout_dir/privileged.log" \
            REPAIRED_MARKER="$layout_dir/repaired" \
            CODEX_HOME="$layout_dir/codex-home" \
            CODEX_MODE=healthy \
            bash "$HELPER" 2>&1)
        layout_status=$?
        set -e
        if [ "$layout_status" -ne 0 ]; then
            fail "$layout_name Codex layout was rejected: $layout_output"
            return 0
        fi

        layout_covered=0
        for layout_granted in $(grep -o '"[^"]*"="read"' "$layout_dir/codex.log" |
            sed 's/^"//; s/"="read"$//'); do
            case "$layout_vendor_bin" in
                "$layout_granted"/*) layout_covered=1 ;;
            esac
        done
        if [ "$layout_covered" -ne 1 ]; then
            fail "$layout_name layout: sandbox grant does not cover the vendored native binary"
        fi
    }

    # Global `npm i -g`: platform package nested under the wrapper's package.
    run_layout_case npm-global \
        "$TMP_DIR/layout-npm-global/lib/node_modules/@openai/codex/bin/codex.js" \
        "$TMP_DIR/layout-npm-global/lib/node_modules/@openai/codex/node_modules/@openai/codex-linux-x64/vendor/x86_64-unknown-linux-musl/bin/codex"

    # Hoisted npm/yarn: platform package is a sibling of the wrapper's package.
    run_layout_case hoisted \
        "$TMP_DIR/layout-hoisted/node_modules/@openai/codex/bin/codex.js" \
        "$TMP_DIR/layout-hoisted/node_modules/@openai/codex-linux-x64/vendor/x86_64-unknown-linux-musl/bin/codex"

    # pnpm/bun: wrapper and platform package live in peer store entries.
    run_layout_case pnpm \
        "$TMP_DIR/layout-pnpm/node_modules/.pnpm/@openai+codex@1.0.0/node_modules/@openai/codex/bin/codex.js" \
        "$TMP_DIR/layout-pnpm/node_modules/.pnpm/@openai+codex-linux-x64@1.0.0/node_modules/@openai/codex-linux-x64/vendor/x86_64-unknown-linux-musl/bin/codex"

    # A standalone native build must not gain a directory grant. The healthy
    # case resolves to $TMP_DIR/codex-package/bin/codex, so that package dir is
    # the path an over-broad implementation would emit.
    if grep -Fq -- "\"$TMP_DIR/codex-package\"=\"read\"" "$TMP_DIR/healthy/codex.log"; then
        fail "non-npm Codex executable was granted an unnecessary directory read"
    fi

    # A standalone native build that happens to sit inside node_modules must
    # not gain a grant either: only the JS wrapper needs the escape hatch, so
    # the wrapper check must stay load-bearing.
    native_nm="$TMP_DIR/native-nm"
    native_nm_bin="$native_nm/node_modules/@openai/codex/bin"
    mkdir -p "$native_nm_bin" "$native_nm/pathbin" \
        "$native_nm/root/etc/apparmor.d" "$native_nm/codex-home"
    printf '%s\n' 'ID=ubuntu' > "$native_nm/root/etc/os-release"
    cp "$FAKE_BIN/codex" "$native_nm_bin/codex"
    chmod +x "$native_nm_bin/codex"
    ln -sf "$native_nm_bin/codex" "$native_nm/pathbin/codex"
    : > "$native_nm/codex.log"
    set +e
    env \
        PATH="$native_nm/pathbin:$FAKE_BIN:$PATH" \
        AUTOSPEC_CODEX_SANDBOX_ROOT="$native_nm/root" \
        AUTOSPEC_CODEX_SANDBOX_TEST_MODE=1 \
        CODEX_LOG="$native_nm/codex.log" \
        SYSCTL_LOG="$native_nm/sysctl.log" \
        PARSER_LOG="$native_nm/parser.log" \
        PRIVILEGED_LOG="$native_nm/privileged.log" \
        REPAIRED_MARKER="$native_nm/repaired" \
        CODEX_HOME="$native_nm/codex-home" \
        CODEX_MODE=healthy \
        bash "$HELPER" >/dev/null 2>&1
    set -e
    if grep -Fq -- "\"$native_nm/node_modules\"=\"read\"" "$native_nm/codex.log"; then
        fail "native Codex build inside node_modules was granted a directory read"
    fi

    # A JS wrapper outside node_modules must not lift the grant to an ancestor
    # such as $HOME, which the credential deny-list would not contain.
    stray_home="$TMP_DIR/stray-home"
    mkdir -p "$stray_home/bin" "$stray_home/root/etc/apparmor.d" "$stray_home/codex-home"
    printf '%s\n' 'ID=ubuntu' > "$stray_home/root/etc/os-release"
    printf '%s\n' '{"name":"home"}' > "$stray_home/package.json"
    cp "$FAKE_BIN/codex" "$stray_home/bin/codex.js"
    chmod +x "$stray_home/bin/codex.js"
    mkdir -p "$stray_home/pathbin"
    ln -sf "$stray_home/bin/codex.js" "$stray_home/pathbin/codex"
    : > "$stray_home/codex.log"
    set +e
    env \
        PATH="$stray_home/pathbin:$FAKE_BIN:$PATH" \
        AUTOSPEC_CODEX_SANDBOX_ROOT="$stray_home/root" \
        AUTOSPEC_CODEX_SANDBOX_TEST_MODE=1 \
        CODEX_LOG="$stray_home/codex.log" \
        SYSCTL_LOG="$stray_home/sysctl.log" \
        PARSER_LOG="$stray_home/parser.log" \
        PRIVILEGED_LOG="$stray_home/privileged.log" \
        REPAIRED_MARKER="$stray_home/repaired" \
        CODEX_HOME="$stray_home/codex-home" \
        CODEX_MODE=healthy \
        bash "$HELPER" >/dev/null 2>&1
    set -e
    if grep -Fq -- "\"$stray_home\"=\"read\"" "$stray_home/codex.log"; then
        fail "wrapper outside node_modules granted read on its whole ancestor directory"
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

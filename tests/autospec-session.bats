#!/usr/bin/env bats
# tests/autospec-session.bats — tests for scripts/autospec-session
# Covers: tmux-missing error, --no-monitor flag, session name pattern,
# harness args pass-through.

bats_require_minimum_version 1.5.0

SESSION_SCRIPT="${BATS_TEST_DIRNAME}/../scripts/autospec-session"

setup() {
    STUB_DIR="$(mktemp -d)"
    export HOME="${BATS_TMPDIR}/home-$$"
    mkdir -p "${HOME}/.autospec/monitors"
}

teardown() {
    rm -rf "${STUB_DIR}" "${HOME}"
}

# ---------------------------------------------------------------------------
# test_errors_when_tmux_missing
# ---------------------------------------------------------------------------

@test "exits 127 when tmux is not installed" {
    # PATH with no tmux — use a stub dir that has no tmux binary.
    empty_path="$(mktemp -d)"

    # Provide uuidgen so argument parsing works before the tmux check.
    printf '#!/usr/bin/env bash\nprintf "ABCDEF12-3456-7890-ABCD-EF1234567890\n"\n' \
        > "${empty_path}/uuidgen"
    chmod +x "${empty_path}/uuidgen"

    run -127 env PATH="${empty_path}" bash "${SESSION_SCRIPT}" claude
    rm -rf "${empty_path}"

    [ "$status" -eq 127 ]
}

# ---------------------------------------------------------------------------
# test_no_monitor_flag_skips_python_spawn
# ---------------------------------------------------------------------------

@test "--no-monitor skips python -m autospec_context_monitor spawn" {
    python3_called="${STUB_DIR}/.python3_called"

    # tmux stub
    cat > "${STUB_DIR}/tmux" <<TMUX
#!/usr/bin/env bash
case "\$1" in
    new-session)    exit 0 ;;
    set-hook)       exit 0 ;;
    attach-session) exit 0 ;;
    attach)         exit 0 ;;
    *)              exit 0 ;;
esac
TMUX
    chmod +x "${STUB_DIR}/tmux"

    # python3 stub records if called
    cat > "${STUB_DIR}/python3" <<PY
#!/usr/bin/env bash
touch "${python3_called}"
exit 0
PY
    chmod +x "${STUB_DIR}/python3"

    # uuidgen stub
    printf '#!/usr/bin/env bash\nprintf "abcdef12-3456-7890-abcd-ef1234567890\n"\n' \
        > "${STUB_DIR}/uuidgen"
    chmod +x "${STUB_DIR}/uuidgen"

    run env PATH="${STUB_DIR}:${PATH}" bash "${SESSION_SCRIPT}" claude --no-monitor

    [ "$status" -eq 0 ]
    [ ! -f "${python3_called}" ]
}

# ---------------------------------------------------------------------------
# test_session_name_starts_with_as_prefix
# ---------------------------------------------------------------------------

@test "session name starts with 'as-' prefix and matches ^as-[a-f0-9]{8}$" {
    session_file="${STUB_DIR}/.session_name"

    # tmux stub captures session name from new-session -d -s NAME ...
    cat > "${STUB_DIR}/tmux" <<TMUX
#!/usr/bin/env bash
case "\$1" in
    new-session)
        # Parse -s flag
        while [[ \$# -gt 0 ]]; do
            if [[ "\$1" == "-s" ]]; then
                printf '%s' "\$2" > "${session_file}"
                break
            fi
            shift
        done
        exit 0
        ;;
    set-hook)       exit 0 ;;
    attach-session) exit 0 ;;
    attach)         exit 0 ;;
    *)              exit 0 ;;
esac
TMUX
    chmod +x "${STUB_DIR}/tmux"

    # python3 stub
    printf '#!/usr/bin/env bash\nexit 0\n' > "${STUB_DIR}/python3"
    chmod +x "${STUB_DIR}/python3"

    # uuidgen stub: produces a known UUID with uppercase + lowercase mix to test tr
    printf '#!/usr/bin/env bash\nprintf "ABCDEF12-3456-7890-ABCD-EF1234567890\n"\n' \
        > "${STUB_DIR}/uuidgen"
    chmod +x "${STUB_DIR}/uuidgen"

    run env PATH="${STUB_DIR}:${PATH}" bash "${SESSION_SCRIPT}" claude --no-monitor

    [ "$status" -eq 0 ]
    [ -f "${session_file}" ]
    session_name="$(cat "${session_file}")"
    [[ "$session_name" =~ ^as-[a-f0-9]{8}$ ]]
}

# ---------------------------------------------------------------------------
# test_passes_harness_args_through_to_tmux
# ---------------------------------------------------------------------------

@test "passes extra harness args through to tmux new-session" {
    args_file="${STUB_DIR}/.tmux_args"

    cat > "${STUB_DIR}/tmux" <<TMUX
#!/usr/bin/env bash
case "\$1" in
    new-session)
        printf '%s\n' "\$@" > "${args_file}"
        exit 0
        ;;
    set-hook)       exit 0 ;;
    attach-session) exit 0 ;;
    attach)         exit 0 ;;
    *)              exit 0 ;;
esac
TMUX
    chmod +x "${STUB_DIR}/tmux"

    printf '#!/usr/bin/env bash\nexit 0\n' > "${STUB_DIR}/python3"
    chmod +x "${STUB_DIR}/python3"

    printf '#!/usr/bin/env bash\nprintf "ABCDEF12-3456-7890-ABCD-EF1234567890\n"\n' \
        > "${STUB_DIR}/uuidgen"
    chmod +x "${STUB_DIR}/uuidgen"

    run env PATH="${STUB_DIR}:${PATH}" bash "${SESSION_SCRIPT}" claude --no-monitor --extra-arg somevalue

    [ "$status" -eq 0 ]
    [ -f "${args_file}" ]
    grep -q "claude" "${args_file}"
    grep -q "somevalue" "${args_file}"
}

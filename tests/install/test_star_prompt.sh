#!/usr/bin/env bash
# Verifies the optional install-time GitHub star prompt without touching a real
# GitHub account. Uses script(1) to provide a pseudo-TTY, matching curl|bash.
#
# Fix for issue #854: the original test fed only one reply (y or n) into the
# pty harness, leaving subsequent interactive prompts in install.sh (the
# auto-rollover prompt and the Claude-hook-mode prompt that follow the star
# prompt in prompt_user_for_auto_rollover) blocking indefinitely on the pty —
# because under script(1) there is no EOF on the pty master side.
#
# Fixes applied:
#   1. Each script(1) invocation is wrapped with `timeout` so a stuck read
#      fails fast (exit 124) rather than hanging indefinitely.
#   2. A full answer sequence is fed for ALL interactive prompts in install.sh
#      so no read -r is left blocking:
#        [1] isolated runtime alias prompt (install_agent_env_aliases)
#        [2] auto-rollover prompt (prompt_user_for_auto_rollover)
#        [3] Claude-hook-mode prompt (prompt_user_for_auto_rollover)
#        [4] optional autospec-db prompt (maybe_prompt_db_module)
#        [5] star prompt (maybe_prompt_star)  ← the reply under test
#        [n] extra blanks as safety buffer
#   3. npm and other slow tools are stubbed in fake_bin so the install
#      completes in seconds without real network activity.
#   4. A post-run orphan assertion confirms no install.sh or script process
#      belonging to this test is still running after all invocations complete.
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

if ! command -v script >/dev/null 2>&1; then
    echo "SKIP: script(1) not found"
    exit 0
fi
if ! script -qfec true /dev/null >/dev/null 2>&1; then
    echo "SKIP: script(1) does not support GNU -qfec pseudo-TTY mode"
    exit 0
fi

# Maximum wall-clock seconds for one install.sh run under script(1).
# With all slow tools stubbed the install completes in ~10s; 45s is generous.
INSTALL_TIMEOUT="${AUTOSPEC_INSTALL_TEST_TIMEOUT:-45}"

tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

setup_fake_home() {
    local fake_home
    fake_home="$(mktemp -d "$tmp_root/home.XXXXXX")"
    mkdir -p "$fake_home/.turbo/repo/claude/skills"
    git -C "$fake_home/.turbo/repo" init -q
    git -C "$fake_home/.turbo/repo" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init
    printf '%s\n' "$fake_home"
}

# setup_fake_bin <gh_log>
# Creates a fake bin directory with stubs for gh, npm, and other tools that
# install.sh may invoke, keeping the test fast and network-free.
setup_fake_bin() {
    local log_file="$1"
    local fake_bin
    fake_bin="$(mktemp -d "$tmp_root/bin.XXXXXX")"

    # gh stub: records invocations for assertions; exits 0 so star-API calls pass.
    cat > "$fake_bin/gh" <<'EOS'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
exit 0
EOS
    chmod +x "$fake_bin/gh"

    # npm stub: pretend every npm install -g succeeds instantly (no network).
    cat > "$fake_bin/npm" <<'EOS'
#!/usr/bin/env bash
exit 0
EOS
    chmod +x "$fake_bin/npm"

    # Stub the oh-my tools so install.sh thinks they're already present
    # (install_npm_ecosystem_package skips the npm call when command_present).
    for _tool in omx omc oh-my-opencode; do
        printf '#!/usr/bin/env bash\nexit 0\n' > "$fake_bin/$_tool"
        chmod +x "$fake_bin/$_tool"
    done

    printf '%s\n' "$fake_bin"
}

# run_install_with_star_reply <star_reply> <output_file> <log_file>
# Answers: y (runtime aliases), n (rollover), n (hook-mode), n (autospec-db),
# <star_reply> (star), + blank buffer.
# Uses a temp file (not command substitution) so trailing newlines are preserved.
run_install_with_star_reply() {
    local star_reply="$1" output_file="$2" log_file="$3"

    local fake_home fake_bin cmd ans_file rc
    fake_home="$(setup_fake_home)"
    fake_bin="$(setup_fake_bin "$log_file")"
    : > "$log_file"

    # HOME is replaced to sandbox what the install WRITES. The Rust toolchain must not be
    # sandboxed with it: rustup reads its default-toolchain from $HOME/.rustup, so a fake HOME
    # made `cargo` inside install.sh fail with "could not choose a version of cargo to run"
    # and the whole install exit 1 on error:runtime-install:build-failed. Pin both homes to
    # the caller's real ones, defaulting the way rustup itself does.
    cmd="cd '$SCRIPT_DIR' && HOME='$fake_home' AUTOSPEC_NO_SHELL_RC_PROMPT=1 AUTOSPEC_SKIP_RUNTIME_BINARY=1 RUSTUP_HOME='${RUSTUP_HOME:-$HOME/.rustup}' CARGO_HOME='${CARGO_HOME:-$HOME/.cargo}' GH_LOG='$log_file' PATH='$fake_bin':\$PATH bash '$SCRIPT_DIR/install.sh' --skill autospec --harness claude"

    # Write answers to a file; printf inside $() would strip trailing newlines,
    # leaving the last read -r in install.sh blocked on the pty (issue #854).
    # Sequence: [1] y=runtime aliases [2] n=rollover [3] n=hook-mode
    # [4] n=autospec-db [5] <reply>=star [6+] buffer
    ans_file="$tmp_root/answers-$$.txt"
    printf 'y\nn\nn\nn\n%s\n\n\n\n\n' "$star_reply" > "$ans_file"

    rc=0
    timeout "$INSTALL_TIMEOUT" script -qfec "$cmd" "$output_file" < "$ans_file" >/dev/null || rc=$?
    if [ "$rc" -ne 0 ]; then
        if [ "$rc" -eq 124 ]; then
            echo "FAIL: install.sh exceeded ${INSTALL_TIMEOUT}s timeout (read -r blocked on pty — issue #854)" >&2
            echo "      Add more answers if new prompts were added to install.sh." >&2
        else
            echo "FAIL: script(1) rc=$rc for star_reply='$star_reply'" >&2
        fi
        return 1
    fi
}

# ── Part 1: TTY install with 'y' answer to star prompt ───────────────────────

yes_output="$tmp_root/yes.out"
yes_log="$tmp_root/yes-gh.log"
run_install_with_star_reply "y" "$yes_output" "$yes_log"
grep -q "Would you like to star https://github.com/berlinguyinca/autospec" "$yes_output" \
    || { echo "FAIL: TTY install did not show star prompt"; cat "$yes_output"; exit 1; }
grep -qF "api -X PUT /user/starred/berlinguyinca/autospec" "$yes_log" \
    || { echo "FAIL: yes answer did not call GitHub star API"; cat "$yes_log"; exit 1; }

# ── Part 2: TTY install with 'n' answer to star prompt ───────────────────────

no_output="$tmp_root/no.out"
no_log="$tmp_root/no-gh.log"
run_install_with_star_reply "n" "$no_output" "$no_log"
if grep -qF "api -X PUT /user/starred/berlinguyinca/autospec" "$no_log"; then
    echo "FAIL: no answer called GitHub star API"
    cat "$no_log"
    exit 1
fi

# ── Part 3: non-TTY install — must not prompt ────────────────────────────────
# stdin is a pipe (not a tty), so install.sh should skip all interactive prompts.

non_tty_home="$(setup_fake_home)"
non_tty_log="$tmp_root/non-tty-gh.log"
non_tty_bin="$(setup_fake_bin "$non_tty_log")"
: > "$non_tty_log"
non_tty_output="$tmp_root/non-tty.out"
# Same environment as the pty case above: sandbox what the install writes, but leave the Rust
# toolchain and the runtime build alone. Without this, install.sh exits 1 and set -e aborts the
# script before either assertion below runs -- a silent failure with no output.
printf 'y\n' | HOME="$non_tty_home" AUTOSPEC_NO_SHELL_RC_PROMPT=1 AUTOSPEC_SKIP_RUNTIME_BINARY=1 \
    RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
    GH_LOG="$non_tty_log" PATH="$non_tty_bin:$PATH" \
    bash "$SCRIPT_DIR/install.sh" --skill autospec --harness claude >"$non_tty_output" 2>&1
if grep -q "Would you like to star" "$non_tty_output"; then
    echo "FAIL: non-TTY install prompted"
    cat "$non_tty_output"
    exit 1
fi
if grep -qF "api -X PUT /user/starred/berlinguyinca/autospec" "$non_tty_log"; then
    echo "FAIL: non-TTY install called GitHub star API"
    cat "$non_tty_log"
    exit 1
fi

# ── Part 4: post-run orphan check ────────────────────────────────────────────
# After all invocations complete, no install.sh process belonging to this test
# run should remain alive. If any are found, the timeout guard was insufficient
# — a regression of issue #854.
if pgrep -f "bash.*install.sh.*--skill autospec" >/dev/null 2>&1; then
    echo "FAIL: orphaned install.sh process found after test completed (issue #854 regression)" >&2
    pgrep -af "bash.*install.sh.*--skill autospec" >&2 || true
    exit 1
fi

echo "PASS"

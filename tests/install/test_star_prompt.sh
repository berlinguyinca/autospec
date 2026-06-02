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
#        [1] auto-rollover prompt (prompt_user_for_auto_rollover)
#        [2] Claude-hook-mode prompt (prompt_user_for_auto_rollover)
#        [3] star prompt (maybe_prompt_star)  ← the reply under test
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
#
# Runs install.sh under script(1), feeding answers for each interactive prompt:
#   [1] auto-rollover? → n (default; not under test)
#   [2] Claude hook mode? → n (default; not under test)
#   [3] star GitHub repo? → <star_reply>  ← the value under test
#   [4+] extra blank lines as safety buffer for any future prompts
#
# This ordering matches the call order in install.sh:
#   prompt_user_for_auto_rollover() → maybe_prompt_star()
# offer_gitignore() is skipped because the repo's .gitignore already contains
# the required entries, so it does not produce a prompt.
run_install_with_star_reply() {
    local star_reply="$1"
    local output_file="$2"
    local log_file="$3"

    local fake_home
    fake_home="$(setup_fake_home)"
    local fake_bin
    fake_bin="$(setup_fake_bin "$log_file")"
    : > "$log_file"

    local cmd
    cmd="cd '$SCRIPT_DIR' && HOME='$fake_home' GH_LOG='$log_file' PATH='$fake_bin':\$PATH bash '$SCRIPT_DIR/install.sh' --skill autospec --harness claude"

    # Write the answer sequence to a temp file to avoid command-substitution
    # stripping of trailing newlines.  The answers are:
    #   [1] n → auto-rollover prompt (prompt_user_for_auto_rollover)
    #   [2] n → Claude hook-mode prompt (prompt_user_for_auto_rollover)
    #   [3] <star_reply> → star prompt (maybe_prompt_star)  ← under test
    #   [4-7] blank lines → safety buffer for any future prompts
    # Using printf without command substitution ensures all trailing newlines
    # are preserved; without them, the last read -r may not return.
    local ans_file="$tmp_root/answers-$$.txt"
    printf 'n\nn\n%s\n\n\n\n\n' "$star_reply" > "$ans_file"

    # Pipe answers into script(1), bounded by INSTALL_TIMEOUT.
    local rc=0
    timeout "$INSTALL_TIMEOUT" script -qfec "$cmd" "$output_file" < "$ans_file" >/dev/null || rc=$?
    if [ "$rc" -ne 0 ]; then
        if [ "$rc" -eq 124 ]; then
            echo "FAIL: install.sh run exceeded ${INSTALL_TIMEOUT}s timeout — a read -r blocked on the pty (issue #854)" >&2
            echo "      This may mean new interactive prompts were added to install.sh; add corresponding" >&2
            echo "      answers to the input sequence in run_install_with_star_reply." >&2
        else
            echo "FAIL: script(1) exited with rc=$rc for star_reply='$star_reply'" >&2
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
printf 'y\n' | HOME="$non_tty_home" GH_LOG="$non_tty_log" PATH="$non_tty_bin:$PATH" \
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

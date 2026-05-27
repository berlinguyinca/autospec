#!/usr/bin/env bash
# Verifies the optional install-time GitHub star prompt without touching a real
# GitHub account. Uses script(1) to provide a pseudo-TTY, matching curl|bash.
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

tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

setup_fake_home() {
    fake_home="$(mktemp -d "$tmp_root/home.XXXXXX")"
    mkdir -p "$fake_home/.turbo/repo/claude/skills"
    git -C "$fake_home/.turbo/repo" init -q
    git -C "$fake_home/.turbo/repo" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init
    printf '%s\n' "$fake_home"
}

setup_fake_gh() {
    log_file="$1"
    fake_bin="$(mktemp -d "$tmp_root/bin.XXXXXX")"
    cat > "$fake_bin/gh" <<'EOS'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
exit 0
EOS
    chmod +x "$fake_bin/gh"
    printf '%s\n' "$fake_bin"
}

run_install_with_reply() {
    reply="$1"
    output_file="$2"
    log_file="$3"
    fake_home="$(setup_fake_home)"
    fake_bin="$(setup_fake_gh "$log_file")"
    : > "$log_file"

    cmd="cd '$SCRIPT_DIR' && HOME='$fake_home' GH_LOG='$log_file' PATH='$fake_bin':\$PATH bash '$SCRIPT_DIR/install.sh' --skill autospec --harness claude"
    printf '%s\n' "$reply" | script -qfec "$cmd" "$output_file" >/dev/null
}

yes_output="$tmp_root/yes.out"
yes_log="$tmp_root/yes-gh.log"
run_install_with_reply "y" "$yes_output" "$yes_log"
grep -q "Would you like to star https://github.com/berlinguyinca/autospec" "$yes_output" \
    || { echo "FAIL: TTY install did not show star prompt"; cat "$yes_output"; exit 1; }
grep -qF "api -X PUT /user/starred/berlinguyinca/autospec" "$yes_log" \
    || { echo "FAIL: yes answer did not call GitHub star API"; cat "$yes_log"; exit 1; }

no_output="$tmp_root/no.out"
no_log="$tmp_root/no-gh.log"
run_install_with_reply "n" "$no_output" "$no_log"
if grep -qF "api -X PUT /user/starred/berlinguyinca/autospec" "$no_log"; then
    echo "FAIL: no answer called GitHub star API"
    cat "$no_log"
    exit 1
fi

non_tty_home="$(setup_fake_home)"
non_tty_log="$tmp_root/non-tty-gh.log"
non_tty_bin="$(setup_fake_gh "$non_tty_log")"
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

echo "PASS"

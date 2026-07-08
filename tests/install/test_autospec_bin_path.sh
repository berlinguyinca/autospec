#!/usr/bin/env bash
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
TEST_HOME="$(mktemp -d -t autospec-install-path.XXXXXX)"
TEMP_SCRIPTS_DIR="$(mktemp -d -t autospec-ephemeral-scripts.XXXXXX)"
trap 'rm -rf "$TEST_HOME" "$TEMP_SCRIPTS_DIR"' EXIT INT TERM

HOME="$TEST_HOME" \
AUTOSPEC_SCRIPTS_DIR="$TEMP_SCRIPTS_DIR" \
AUTOSPEC_SKIP_SYSTEM_TOOLS=1 \
AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
AUTOSPEC_SKIP_SUPERPOWERS=1 \
AUTOSPEC_SKIP_OH_MY_CODEX=1 \
AUTOSPEC_SKIP_OH_MY_OPENCODE=1 \
AUTOSPEC_SKIP_OH_MY_CLAUDE=1 \
AUTOSPEC_NO_STAR_PROMPT=1 \
CI=1 \
bash "$SCRIPT_DIR/install.sh" --skill autospec-autonomous --harness codex >/tmp/autospec-install-path.out 2>&1

rm -rf "$TEMP_SCRIPTS_DIR"

[ -d "$TEST_HOME/.autospec/bin" ] || {
    echo "FAIL: ~/.autospec/bin was not created"
    cat /tmp/autospec-install-path.out
    exit 1
}

[ -f "$TEST_HOME/.autospec/env" ] || {
    echo "FAIL: ~/.autospec/env was not created"
    cat /tmp/autospec-install-path.out
    exit 1
}

grep -q 'AUTOSPEC_BIN_DIR="$HOME/.autospec/bin"' "$TEST_HOME/.autospec/env" || {
    echo "FAIL: ~/.autospec/env does not export the autospec bin directory"
    cat "$TEST_HOME/.autospec/env"
    exit 1
}

grep -qxF '. "$HOME/.autospec/env"' "$TEST_HOME/.zshrc" || {
    echo "FAIL: ~/.zshrc does not source ~/.autospec/env"
    cat /tmp/autospec-install-path.out
    exit 1
}

grep -qxF '. "$HOME/.autospec/env"' "$TEST_HOME/.bashrc" || {
    echo "FAIL: ~/.bashrc does not source ~/.autospec/env"
    cat /tmp/autospec-install-path.out
    exit 1
}

for command in autospec-autonomous autospec-autonomous-status autospec-autonomous-timeline autospec-autonomous-monitor autospec-autonomous-logs autospec-autonomous-watch autospec-autonomous-stop autospec-autonomous-restart; do
    [ -x "$TEST_HOME/.autospec/bin/$command" ] || {
        echo "FAIL: $command wrapper was not installed"
        ls -la "$TEST_HOME/.autospec/bin" || true
        cat /tmp/autospec-install-path.out
        exit 1
    }
done

if grep -R '/tmp/' "$TEST_HOME/.autospec/bin"/autospec-autonomous*; then
    echo "FAIL: autospec-autonomous wrappers contain a literal /tmp path"
    exit 1
fi

if grep -R '^exec "/' "$TEST_HOME/.autospec/bin"/autospec-autonomous*; then
    echo "FAIL: autospec-autonomous wrappers contain an absolute exec target"
    exit 1
fi

if grep -R '^export HOME=' "$TEST_HOME/.autospec/bin"/autospec-autonomous*; then
    echo "FAIL: autospec-autonomous wrappers pin HOME at install time"
    exit 1
fi

for command in autospec-autonomous autospec-autonomous-status autospec-autonomous-timeline autospec-autonomous-monitor autospec-autonomous-logs autospec-autonomous-watch autospec-autonomous-stop autospec-autonomous-restart; do
    grep -qF 'exec "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-autonomous.sh"' "$TEST_HOME/.autospec/bin/$command" || {
        echo "FAIL: $command does not use the runtime-resolving launcher"
        cat "$TEST_HOME/.autospec/bin/$command"
        exit 1
    }
done


# Regression: an already-installed stale wrapper from the pre-fix generator must
# be healed during --update, not left broken until a clean reinstall.
cat > "$TEST_HOME/.autospec/bin/autospec-autonomous-status" <<'STALE'
#!/usr/bin/env bash
set -eu
exec "/tmp/gone/autospec-autonomous.sh" status "$@"
STALE
chmod +x "$TEST_HOME/.autospec/bin/autospec-autonomous-status"

HOME="$TEST_HOME" \
AUTOSPEC_SCRIPTS_DIR="$TEMP_SCRIPTS_DIR" \
AUTOSPEC_SKIP_SYSTEM_TOOLS=1 \
AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
AUTOSPEC_SKIP_SUPERPOWERS=1 \
AUTOSPEC_SKIP_OH_MY_CODEX=1 \
AUTOSPEC_SKIP_OH_MY_OPENCODE=1 \
AUTOSPEC_SKIP_OH_MY_CLAUDE=1 \
AUTOSPEC_NO_STAR_PROMPT=1 \
CI=1 \
bash "$SCRIPT_DIR/install.sh" --skill autospec-autonomous --harness codex --update >/tmp/autospec-install-heal.out 2>&1

grep -q 'heal_autonomous_operator_wrappers: healed' /tmp/autospec-install-heal.out || {
    echo "FAIL: --update did not log healed stale autonomous wrapper"
    cat /tmp/autospec-install-heal.out
    exit 1
}

grep -qF 'exec "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-autonomous.sh" status "$@"' "$TEST_HOME/.autospec/bin/autospec-autonomous-status" || {
    echo "FAIL: stale autospec-autonomous-status wrapper was not rewritten to runtime-resolving form"
    cat "$TEST_HOME/.autospec/bin/autospec-autonomous-status"
    exit 1
}

HOME="$TEST_HOME" "$TEST_HOME/.autospec/bin/autospec-autonomous-status" --json >/tmp/autospec-autonomous-status-healed.json || {
    echo "FAIL: healed autospec-autonomous-status command did not execute"
    cat /tmp/autospec-autonomous-status-healed.json 2>/dev/null || true
    cat /tmp/autospec-install-heal.out
    exit 1
}

grep -q '"running":false' /tmp/autospec-autonomous-status-healed.json || {
    echo "FAIL: healed autospec-autonomous-status did not resolve under HOME/.autospec"
    cat /tmp/autospec-autonomous-status-healed.json
    exit 1
}

HOME="$TEST_HOME" \
AUTOSPEC_SKIP_SYSTEM_TOOLS=1 \
AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
AUTOSPEC_SKIP_SUPERPOWERS=1 \
AUTOSPEC_SKIP_OH_MY_CODEX=1 \
AUTOSPEC_SKIP_OH_MY_OPENCODE=1 \
AUTOSPEC_SKIP_OH_MY_CLAUDE=1 \
AUTOSPEC_NO_STAR_PROMPT=1 \
CI=1 \
bash "$SCRIPT_DIR/install.sh" --skill autospec-autonomous --harness codex --update >/tmp/autospec-install-heal-noop.out 2>&1

if grep -q 'heal_autonomous_operator_wrappers: healed' /tmp/autospec-install-heal-noop.out; then
    echo "FAIL: heal step rewrote already-correct autonomous wrapper"
    cat /tmp/autospec-install-heal-noop.out
    exit 1
fi

HOME="$TEST_HOME" "$TEST_HOME/.autospec/bin/autospec-autonomous-status" --json >/tmp/autospec-autonomous-status.json || {
    echo "FAIL: autospec-autonomous-status command did not run after ephemeral scripts dir deletion"
    cat /tmp/autospec-autonomous-status.json 2>/dev/null || true
    cat /tmp/autospec-install-path.out
    exit 1
}

grep -q '"running":false' /tmp/autospec-autonomous-status.json || {
    echo "FAIL: autospec-autonomous status did not report stopped state"
    cat /tmp/autospec-autonomous-status.json
    exit 1
}

echo "PASS"

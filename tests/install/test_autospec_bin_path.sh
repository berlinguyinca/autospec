#!/usr/bin/env bash
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
TEST_HOME="$(mktemp -d -t autospec-install-path.XXXXXX)"
trap 'rm -rf "$TEST_HOME"' EXIT INT TERM

HOME="$TEST_HOME" \
AUTOSPEC_SKIP_SYSTEM_TOOLS=1 \
AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
AUTOSPEC_SKIP_SUPERPOWERS=1 \
AUTOSPEC_SKIP_OH_MY_CODEX=1 \
AUTOSPEC_SKIP_OH_MY_OPENCODE=1 \
AUTOSPEC_SKIP_OH_MY_CLAUDE=1 \
AUTOSPEC_NO_STAR_PROMPT=1 \
CI=1 \
bash "$SCRIPT_DIR/install.sh" --skill autospec-autonomous --harness codex >/tmp/autospec-install-path.out 2>&1

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

for command in autospec-autonomous autospec-autonomous-status autospec-autonomous-logs autospec-autonomous-watch autospec-autonomous-stop autospec-autonomous-restart; do
    [ -x "$TEST_HOME/.autospec/bin/$command" ] || {
        echo "FAIL: $command wrapper was not installed"
        ls -la "$TEST_HOME/.autospec/bin" || true
        cat /tmp/autospec-install-path.out
        exit 1
    }
done

"$TEST_HOME/.autospec/bin/autospec-autonomous" status --json >/tmp/autospec-autonomous-status.json || {
    echo "FAIL: autospec-autonomous status command did not run"
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

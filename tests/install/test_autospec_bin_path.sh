#!/usr/bin/env bash
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
TEST_CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
TEST_RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
TEST_HOME="$(mktemp -d -t autospec-install-path.XXXXXX)"
TEMP_SCRIPTS_DIR="$(mktemp -d -t autospec-ephemeral-scripts.XXXXXX)"
trap 'rm -rf "$TEST_HOME" "$TEMP_SCRIPTS_DIR"' EXIT INT TERM

HOME="$TEST_HOME" \
CARGO_HOME="$TEST_CARGO_HOME" \
RUSTUP_HOME="$TEST_RUSTUP_HOME" \
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

grep -q 'install_autospec_runtime_binary: installed' /tmp/autospec-install-path.out || {
    echo "FAIL: installer did not install the Rust autospec runtime"
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

for command in autospec-autonomous autospec-autonomous-start autospec-autonomous-status autospec-autonomous-list autospec-autonomous-timeline autospec-autonomous-monitor autospec-autonomous-supervise autospec-autonomous-logs autospec-autonomous-watch autospec-autonomous-cleanup autospec-autonomous-stop autospec-autonomous-restart; do
    [ -x "$TEST_HOME/.autospec/bin/$command" ] || {
        echo "FAIL: $command wrapper was not installed"
        ls -la "$TEST_HOME/.autospec/bin" || true
        cat /tmp/autospec-install-path.out
        exit 1
    }
done

for helper in autospec-autonomous-launcher.sh autonomous-runtime-refresh.sh autospec-runtime-install.sh; do
    [ -x "$TEST_HOME/.autospec/scripts/$helper" ] || {
        echo "FAIL: autonomous runtime helper $helper was not installed"
        ls -la "$TEST_HOME/.autospec/scripts" || true
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

for command in autospec-autonomous autospec-autonomous-start autospec-autonomous-status autospec-autonomous-list autospec-autonomous-timeline autospec-autonomous-monitor autospec-autonomous-supervise autospec-autonomous-logs autospec-autonomous-watch autospec-autonomous-cleanup autospec-autonomous-stop autospec-autonomous-restart; do
    grep -qF 'autospec-autonomous-launcher.sh' "$TEST_HOME/.autospec/bin/$command" || {
        echo "FAIL: $command does not delegate to the shared autonomous launcher"
        cat "$TEST_HOME/.autospec/bin/$command"
        exit 1
    }
    [ "$(grep -cF 'AUTOSPEC_WRAPPER_BIN_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"' "$TEST_HOME/.autospec/bin/$command")" -eq 1 ] || {
        echo "FAIL: $command does not resolve its own bin directory exactly once"
        cat "$TEST_HOME/.autospec/bin/$command"
        exit 1
    }
    [ "$(grep -cF '*) PATH="$AUTOSPEC_WRAPPER_BIN_DIR:$PATH"; export PATH ;;' "$TEST_HOME/.autospec/bin/$command")" -eq 1 ] || {
        echo "FAIL: $command does not prepend its own bin directory exactly once"
        cat "$TEST_HOME/.autospec/bin/$command"
        exit 1
    }
done

# Premerge-gate scanner shims (issue: autospec#1693 / autotrade#1350 Part 1 systemic):
# the gate resolves autospec-qa / autospec-secaudit with `command -v`, so the
# installer must place PATH-resolvable shims that run the skill headlessly via omx.
for scanner in autospec-qa autospec-secaudit; do
    shim="$TEST_HOME/.autospec/bin/$scanner"
    [ -x "$shim" ] || {
        echo "FAIL: $scanner premerge-gate shim was not installed"
        ls -la "$TEST_HOME/.autospec/bin" || true
        cat /tmp/autospec-install-path.out
        exit 1
    }
    grep -qF "\$$scanner" "$shim" || {
        echo "FAIL: $scanner shim does not invoke the \$$scanner skill"
        cat "$shim"
        exit 1
    }
    grep -qF 'omx exec' "$shim" || {
        echo "FAIL: $scanner shim does not run the skill via omx exec"
        cat "$shim"
        exit 1
    }
    # Must not FAKE presence by assigning *_PRESENT_OVERRIDE (a comment mentioning
    # that it avoids the override is fine; an actual assignment is not).
    if grep -qE 'PRESENT_OVERRIDE[[:space:]]*=' "$shim"; then
        echo "FAIL: $scanner shim fakes presence via *_PRESENT_OVERRIDE instead of running the scan"
        cat "$shim"
        exit 1
    fi
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
CARGO_HOME="$TEST_CARGO_HOME" \
RUSTUP_HOME="$TEST_RUSTUP_HOME" \
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

grep -qF 'autospec-autonomous-launcher.sh" status "$@"' "$TEST_HOME/.autospec/bin/autospec-autonomous-status" || {
    echo "FAIL: stale autospec-autonomous-status wrapper was not rewritten to launcher form"
    cat "$TEST_HOME/.autospec/bin/autospec-autonomous-status"
    exit 1
}

HOME="$TEST_HOME" PATH=/usr/bin:/bin "$TEST_HOME/.autospec/bin/autospec-autonomous-status" --json >/tmp/autospec-autonomous-status-healed.json || {
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
CARGO_HOME="$TEST_CARGO_HOME" \
RUSTUP_HOME="$TEST_RUSTUP_HOME" \
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

HOME="$TEST_HOME" PATH=/usr/bin:/bin "$TEST_HOME/.autospec/bin/autospec-autonomous-status" --json >/tmp/autospec-autonomous-status.json || {
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

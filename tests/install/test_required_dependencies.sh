#!/usr/bin/env bash
set -eu

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP_DIR="$(mktemp -d)"
ISOLATED_BIN="$TMP_DIR/bin"
FAKE_HOME="$TMP_DIR/home"
ORIGINAL_HOME="$HOME"
ORIGINAL_PATH="$PATH"
failures=0

cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    failures=$((failures + 1))
}

mkdir -p "$ISOLATED_BIN" "$FAKE_HOME"

# Preserve normal shell utilities while excluding the dependencies under test.
old_ifs="$IFS"
IFS=:
for command_dir in $ORIGINAL_PATH; do
    [ -d "$command_dir" ] || continue
    for command_path in "$command_dir"/*; do
        [ -e "$command_path" ] || continue
        command_name="$(basename "$command_path")"
        case "$command_name" in
            git|curl|cargo|python3|gh|jq|codex|claude|opencode|brew|apt-get|dnf|yum|pacman|apk|winget|choco|scoop|sudo) continue ;;
        esac
        [ -e "$ISOLATED_BIN/$command_name" ] && continue
        ln -s "$command_path" "$ISOLATED_BIN/$command_name" 2>/dev/null || true
    done
done
IFS="$old_ifs"

dry_output=$(AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    bash "$ROOT/install.sh" --dry-run --skill autospec --harness codex 2>&1 || true)
required_line=$(printf '%s\n' "$dry_output" | grep -n 'ensure_required_system_tools' | head -1 | cut -d: -f1 || true)
build_line=$(printf '%s\n' "$dry_output" | grep -n 'install_autospec_runtime_binary: cargo build' | head -1 | cut -d: -f1 || true)
if [ -z "$required_line" ] || [ -z "$build_line" ] || [ "$required_line" -ge "$build_line" ]; then
    fail "required tools were not ensured before cargo build"
fi
node_line=$(printf '%s\n' "$dry_output" | grep -n 'ensure_system_tools: would ensure node' | head -1 | cut -d: -f1 || true)
codex_line=$(printf '%s\n' "$dry_output" | grep -n 'ensure_system_tools: would ensure codex' | head -1 | cut -d: -f1 || true)
if [ -z "$node_line" ] || [ -z "$codex_line" ] || [ "$node_line" -ge "$codex_line" ]; then
    fail "Node/npm were not ensured before npm-based harness CLIs"
fi
case "$dry_output" in
    *"required missing: not verified (dry-run)"*) ;;
    *) fail "dry-run reported an unverified required-dependency success" ;;
esac

set +e
missing_output=$(HOME="$FAKE_HOME" PATH="$ISOLATED_BIN" \
    AUTOSPEC_SKIP_SYSTEM_TOOLS=1 \
    AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
    AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    AUTOSPEC_NO_DB_PROMPT=1 \
    AUTOSPEC_NO_STAR_PROMPT=1 \
    bash "$ROOT/install.sh" --skill autospec --harness codex 2>&1)
missing_status=$?
set -e

if [ "$missing_status" -eq 0 ]; then
    fail "missing required tools did not fail installation"
fi
case "$missing_output" in
    *"required missing:"*) ;;
    *) fail "missing dependency report has no required-missing category" ;;
esac
for tool in git curl cargo python3 gh jq; do
    case "$missing_output" in
        *"$tool"*) ;;
        *) fail "missing dependency report omitted $tool" ;;
    esac
done
case "$missing_output" in
    *"install_autospec_runtime_binary: building"*) fail "cargo build ran after required verification failed" ;;
esac

set +e
no_manager_output=$(HOME="$FAKE_HOME" PATH="$ISOLATED_BIN" \
    AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
    AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    AUTOSPEC_NO_DB_PROMPT=1 \
    AUTOSPEC_NO_STAR_PROMPT=1 \
    bash "$ROOT/install.sh" --skill autospec --harness codex 2>&1)
no_manager_status=$?
set -e
if [ "$no_manager_status" -eq 0 ]; then
    fail "installation succeeded without required tools or a package manager"
fi
case "$no_manager_output" in
    *"no supported package manager was available"*) ;;
    *) fail "required failure did not explain the missing package-manager path" ;;
esac

cat > "$ISOLATED_BIN/apt-get" <<'SHIM'
#!/usr/bin/env bash
exit 1
SHIM
chmod +x "$ISOLATED_BIN/apt-get"
set +e
no_sudo_output=$(HOME="$FAKE_HOME" PATH="$ISOLATED_BIN" \
    AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
    AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    AUTOSPEC_NO_DB_PROMPT=1 \
    AUTOSPEC_NO_STAR_PROMPT=1 \
    bash "$ROOT/install.sh" --skill autospec --harness codex 2>&1)
no_sudo_status=$?
set -e
rm -f "$ISOLATED_BIN/apt-get"
if [ "$no_sudo_status" -eq 0 ]; then
    fail "installation succeeded when system packages needed unavailable sudo"
fi
case "$no_sudo_output" in
    *"apt-get needs root privileges, but sudo is unavailable"*) ;;
    *) fail "required failure did not explain the unavailable sudo path" ;;
esac

# Make the non-Python core commands available while keeping all harnesses
# absent. Cargo writes to a temporary target directory so this test can run in
# parallel with real builds.
for tool in git curl gh jq; do
    printf '#!/usr/bin/env bash\nexit 0\n' > "$ISOLATED_BIN/$tool"
    chmod +x "$ISOLATED_BIN/$tool"
done
TEST_CARGO_TARGET_DIR="$TMP_DIR/cargo-target"
cat > "$ISOLATED_BIN/cargo" <<SHIM
#!/usr/bin/env bash
mkdir -p "$TEST_CARGO_TARGET_DIR/release"
printf '#!/usr/bin/env bash\nexit 0\n' > "$TEST_CARGO_TARGET_DIR/release/autospec"
chmod +x "$TEST_CARGO_TARGET_DIR/release/autospec"
exit 0
SHIM
chmod +x "$ISOLATED_BIN/cargo"
cat > "$ISOLATED_BIN/codex" <<'SHIM'
#!/usr/bin/env bash
exit 0
SHIM
chmod +x "$ISOLATED_BIN/codex"

# WinGet updates the persistent Windows PATH, not the already-running Git Bash
# process. Simulate Python landing outside the original PATH and require the
# same installer process to import the refreshed PATH and expose python3.
WINDOWS_HOME="$TMP_DIR/windows-home"
WINDOWS_BIN="$TMP_DIR/windows-bin"
mkdir -p "$WINDOWS_HOME" "$WINDOWS_BIN"
cat > "$ISOLATED_BIN/winget" <<SHIM
#!/usr/bin/env bash
case "\$*" in
    *Python.Python.3.12*)
        printf '#!/usr/bin/env bash\nexit 0\n' > "$WINDOWS_BIN/python"
        chmod +x "$WINDOWS_BIN/python"
        ;;
esac
exit 0
SHIM
cat > "$ISOLATED_BIN/powershell.exe" <<'SHIM'
#!/usr/bin/env bash
printf 'C:\\Program Files\\Python\r\n'
SHIM
cat > "$ISOLATED_BIN/cygpath" <<SHIM
#!/usr/bin/env bash
printf '%s\n' "$WINDOWS_BIN"
SHIM
chmod +x "$ISOLATED_BIN/winget" "$ISOLATED_BIN/powershell.exe" "$ISOLATED_BIN/cygpath"

set +e
windows_output=$(HOME="$WINDOWS_HOME" PATH="$ISOLATED_BIN" \
    CARGO_TARGET_DIR="$TEST_CARGO_TARGET_DIR" \
    AUTOSPEC_REQUIRED_SYSTEM_TOOLS=python3 \
    AUTOSPEC_SYSTEM_TOOLS=true \
    AUTOSPEC_HARNESS_TOOLS=codex \
    AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
    AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    AUTOSPEC_NO_DB_PROMPT=1 \
    AUTOSPEC_NO_STAR_PROMPT=1 \
    TURBO_REPO_DIR="$TMP_DIR/windows-turbo" \
    bash "$ROOT/install.sh" --skill autospec --harness codex 2>&1)
windows_status=$?
set -e
if [ "$windows_status" -ne 0 ]; then
    fail "WinGet-installed Python was not discovered in the same Git Bash process: $windows_output"
fi
if [ ! -x "$WINDOWS_HOME/.autospec/bin/python3" ]; then
    fail "WinGet-installed Python did not receive a local python3 command alias"
fi
rm -f "$ISOLATED_BIN/winget" "$ISOLATED_BIN/powershell.exe" "$ISOLATED_BIN/cygpath"
rm -f "$ISOLATED_BIN/codex"

printf '#!/usr/bin/env bash\nexit 0\n' > "$ISOLATED_BIN/python3"
chmod +x "$ISOLATED_BIN/python3"
EVAL_MARKER="$TMP_DIR/eval-marker"
MALICIOUS_TOOL="\$(touch\${IFS}$EVAL_MARKER)"

set +e
harness_output=$(HOME="$FAKE_HOME" PATH="$ISOLATED_BIN" \
    CARGO_TARGET_DIR="$TEST_CARGO_TARGET_DIR" \
    AUTOSPEC_SKIP_SYSTEM_TOOLS=1 \
    AUTOSPEC_SYSTEM_TOOLS="$MALICIOUS_TOOL" \
    AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
    AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    AUTOSPEC_NO_DB_PROMPT=1 \
    AUTOSPEC_NO_STAR_PROMPT=1 \
    bash "$ROOT/install.sh" --skill autospec --harness codex 2>&1)
harness_status=$?
set -e

if [ "$harness_status" -eq 0 ]; then
    fail "installation succeeded without codex, claude, or opencode"
fi
case "$harness_output" in
    *"required harness missing: codex claude opencode"*) ;;
    *) fail "harness failure did not list the required harness alternatives" ;;
esac
if [ -e "$EVAL_MARKER" ]; then
    fail "dependency names from environment overrides were evaluated as shell code"
fi

cat > "$ISOLATED_BIN/codex" <<'SHIM'
#!/usr/bin/env bash
exit 0
SHIM
chmod +x "$ISOLATED_BIN/codex"
set +e
success_output=$(HOME="$FAKE_HOME" PATH="$ISOLATED_BIN" \
    CARGO_TARGET_DIR="$TEST_CARGO_TARGET_DIR" \
    AUTOSPEC_SKIP_SYSTEM_TOOLS=1 \
    AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
    AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    AUTOSPEC_NO_DB_PROMPT=1 \
    AUTOSPEC_NO_STAR_PROMPT=1 \
    TURBO_REPO_DIR="$TMP_DIR/turbo" \
    bash "$ROOT/install.sh" --skill autospec --harness codex 2>&1)
success_status=$?
set -e
if [ "$success_status" -ne 0 ]; then
    fail "installation failed with all core commands and Codex available"
fi
case "$success_output" in
    *"required missing: none"*) ;;
    *) fail "successful install omitted the verified dependency summary" ;;
esac

if [ "$failures" -ne 0 ]; then
    exit 1
fi

printf 'PASS\n'

#!/usr/bin/env bash
set -eu

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP_DIR="$(mktemp -d)"
ISOLATED_BIN="$TMP_DIR/bin"
FAKE_HOME="$TMP_DIR/home"
ORIGINAL_HOME="$HOME"
ORIGINAL_PATH="$PATH"
ORIGINAL_UID="$(id -u)"
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
            git|curl|cargo|python3|gh|jq|codex|claude|opencode|gitleaks|semgrep|trivy|license-checker|brew|apt-get|dnf|yum|pacman|apk|winget|choco|scoop|sudo|npm|pip|pip3|pipx|uv) continue ;;
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
cat > "$ISOLATED_BIN/git" <<'SHIM'
#!/usr/bin/env bash
if [ "${1:-}" = "-C" ] && [ "${3:-}" = "rev-parse" ] && [ "${4:-}" = "--show-toplevel" ]; then
    (CDPATH='' cd -- "$2" && pwd -P)
elif [ "${1:-}" = "-C" ] && [ "${3:-}" = "rev-parse" ] && [ "${4:-}" = "--verify" ] && [ "${5:-}" = "HEAD" ]; then
    printf '0000000000000000000000000000000000000000\n'
elif [ "${1:-}" = "-C" ] && [ "${3:-}" = "ls-files" ]; then
    exit 0
else
    exit 0
fi
SHIM
chmod +x "$ISOLATED_BIN/git"
TEST_CARGO_TARGET_DIR="$TMP_DIR/cargo-target"
cat > "$ISOLATED_BIN/cargo" <<SHIM
#!/usr/bin/env bash
mkdir -p "\${CARGO_TARGET_DIR:-$TEST_CARGO_TARGET_DIR}/release"
printf '#!/usr/bin/env bash\nexit 0\n' > "\${CARGO_TARGET_DIR:-$TEST_CARGO_TARGET_DIR}/release/autospec"
chmod +x "\${CARGO_TARGET_DIR:-$TEST_CARGO_TARGET_DIR}/release/autospec"
exit 0
SHIM
chmod +x "$ISOLATED_BIN/cargo"
cat > "$ISOLATED_BIN/codex" <<'SHIM'
#!/usr/bin/env bash
exit 0
SHIM
chmod +x "$ISOLATED_BIN/codex"
printf '#!/usr/bin/env bash\nexit 0\n' > "$ISOLATED_BIN/python3"
chmod +x "$ISOLATED_BIN/python3"

# Autonomous execution fails closed unless all executor scanners are installed
# and discoverable. Exercise the real installer with package-manager shims that
# materialize commands only when the expected fallback is attempted.
SCANNER_LOG="$TMP_DIR/scanner-install.log"
rm -f "$ISOLATED_BIN/id"
cat > "$ISOLATED_BIN/id" <<SHIM
#!/usr/bin/env bash
[ "\${1:-}" = "-u" ] && printf '%s\n' "$ORIGINAL_UID"
SHIM
cat > "$ISOLATED_BIN/sudo" <<SHIM
#!/usr/bin/env bash
printf 'sudo %s\n' "\$*" >> "$SCANNER_LOG"
exec "\$@"
SHIM
cat > "$ISOLATED_BIN/apt-get" <<SHIM
#!/usr/bin/env bash
printf 'apt-get %s\n' "\$*" >> "$SCANNER_LOG"
for package in "\$@"; do
    case "\$package" in
        gitleaks|semgrep|trivy)
            printf '#!/usr/bin/env bash\nexit 0\n' > "$ISOLATED_BIN/\$package"
            chmod +x "$ISOLATED_BIN/\$package"
            ;;
    esac
done
exit 0
SHIM
cat > "$ISOLATED_BIN/npm" <<SHIM
#!/usr/bin/env bash
printf 'npm %s\n' "\$*" >> "$SCANNER_LOG"
case "\$*" in
    *license-checker*)
        printf '#!/usr/bin/env bash\nexit 0\n' > "$ISOLATED_BIN/license-checker"
        chmod +x "$ISOLATED_BIN/license-checker"
        ;;
esac
exit 0
SHIM
cat > "$ISOLATED_BIN/pipx" <<SHIM
#!/usr/bin/env bash
printf 'pipx %s\n' "\$*" >> "$SCANNER_LOG"
case "\$*" in
    *semgrep*)
        printf '#!/usr/bin/env bash\nexit 0\n' > "$ISOLATED_BIN/semgrep"
        chmod +x "$ISOLATED_BIN/semgrep"
        ;;
esac
exit 0
SHIM
chmod +x "$ISOLATED_BIN/id" "$ISOLATED_BIN/sudo" "$ISOLATED_BIN/apt-get" "$ISOLATED_BIN/npm" "$ISOLATED_BIN/pipx"

set +e
scanner_output=$(HOME="$FAKE_HOME" PATH="$ISOLATED_BIN" \
    CARGO_TARGET_DIR="$TEST_CARGO_TARGET_DIR" \
    AUTOSPEC_REQUIRED_SYSTEM_TOOLS="git bash curl cargo python3 gh jq npm" \
    AUTOSPEC_SYSTEM_TOOLS=true \
    AUTOSPEC_HARNESS_TOOLS=codex \
    AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
    AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    AUTOSPEC_NO_DB_PROMPT=1 \
    AUTOSPEC_NO_STAR_PROMPT=1 \
    TURBO_REPO_DIR="$TMP_DIR/scanner-turbo" \
    bash "$ROOT/install.sh" --disable-auto-rollover --skill autospec --harness codex 2>&1)
scanner_status=$?
set -e
if [ "$scanner_status" -ne 0 ]; then
    fail "autonomous scanner installation did not provision and verify every required scanner: $scanner_output"
fi
for scanner in gitleaks semgrep trivy license-checker; do
    if [ ! -x "$ISOLATED_BIN/$scanner" ]; then
        fail "autonomous scanner installation did not expose $scanner on PATH"
    fi
done
for scanner in gitleaks trivy; do
    if ! grep -q "^sudo apt-get install -y $scanner$" "$SCANNER_LOG"; then
        fail "autonomous scanner installation did not attempt $scanner through the approved sudo APT fallback"
    fi
done
if ! grep -q '^pipx install semgrep$' "$SCANNER_LOG"; then
    fail "autonomous scanner installation did not attempt semgrep through pipx"
fi
if ! grep -q '^npm install -g license-checker$' "$SCANNER_LOG"; then
    fail "autonomous scanner installation did not attempt license-checker through npm"
fi

rm -f "$ISOLATED_BIN/semgrep"
set +e
skipped_scanner_output=$(HOME="$FAKE_HOME" PATH="$ISOLATED_BIN" \
    CARGO_TARGET_DIR="$TEST_CARGO_TARGET_DIR" \
    AUTOSPEC_REQUIRED_SYSTEM_TOOLS="git bash curl cargo python3 gh jq npm" \
    AUTOSPEC_SKIP_ENSURE_TOOL_SEMGREP=1 \
    AUTOSPEC_SYSTEM_TOOLS=true \
    AUTOSPEC_HARNESS_TOOLS=codex \
    AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
    AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    AUTOSPEC_NO_DB_PROMPT=1 \
    AUTOSPEC_NO_STAR_PROMPT=1 \
    TURBO_REPO_DIR="$TMP_DIR/skipped-scanner-turbo" \
    bash "$ROOT/install.sh" --disable-auto-rollover --skill autospec --harness codex 2>&1)
skipped_scanner_status=$?
set -e
if [ "$skipped_scanner_status" -eq 0 ]; then
    fail "per-tool scanner skip bypassed required scanner verification"
fi
if ! printf '%s\n' "$skipped_scanner_output" | grep -qx 'error: required missing: semgrep'; then
    fail "per-tool scanner skip did not report the exact missing scanner: $skipped_scanner_output"
fi
printf '#!/usr/bin/env bash\nexit 0\n' > "$ISOLATED_BIN/semgrep"
chmod +x "$ISOLATED_BIN/semgrep"
rm -f "$ISOLATED_BIN/python3" "$ISOLATED_BIN/id" "$ISOLATED_BIN/sudo" "$ISOLATED_BIN/apt-get" "$ISOLATED_BIN/pipx"
cat > "$ISOLATED_BIN/id" <<SHIM
#!/usr/bin/env bash
[ "\${1:-}" = "-u" ] && printf '%s\n' "$ORIGINAL_UID"
SHIM
chmod +x "$ISOLATED_BIN/id"

# WinGet updates the persistent Windows PATH, not the already-running Git Bash
# process. Simulate Python landing outside the original PATH and require the
# same installer process to import the refreshed PATH and expose python3.
WINDOWS_HOME="$TMP_DIR/windows-home"
WINDOWS_BIN="$TMP_DIR/windows-bin"
WINDOWS_FIND_MARKER="$TMP_DIR/windows-find-marker"
mkdir -p "$WINDOWS_HOME" "$WINDOWS_BIN"
cat > "$ISOLATED_BIN/winget" <<SHIM
#!/usr/bin/env bash
case "\$*" in
    *Python.Python.3.12*)
        printf '#!/usr/bin/env bash\n[ "\$(command -v find)" != "$WINDOWS_BIN/find" ] || touch "$WINDOWS_FIND_MARKER"\nexit 0\n' > "$WINDOWS_BIN/python"
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
cat > "$WINDOWS_BIN/find" <<SHIM
#!/usr/bin/env bash
touch "$WINDOWS_FIND_MARKER"
exit 99
SHIM
chmod +x "$WINDOWS_BIN/find"
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
if [ -e "$WINDOWS_FIND_MARKER" ]; then
    fail "refreshed Windows PATH shadowed Git Bash's POSIX find command"
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

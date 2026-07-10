#!/usr/bin/env bats
# tests/unit/install-db-module.bats — top-level install.sh optional autospec-db prompt/update contract.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    INSTALL_SH="$REPO_ROOT/install.sh"
    FAKE_HOME="$(mktemp -d)"
    export HOME="$FAKE_HOME"
    BIN_DIR="$FAKE_HOME/bin"
    LOG="$FAKE_HOME/curl.log"
    mkdir -p "$BIN_DIR"
    cat > "$BIN_DIR/curl" <<'SH'
#!/usr/bin/env bash
printf 'curl %s\n' "$*" >> "$CURL_LOG"
if [ "${CURL_FAIL:-0}" = "1" ]; then
  exit 22
fi
out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) shift; out="${1:-}" ;;
  esac
  shift || break
done
if [ -n "$out" ]; then
  cat > "$out" <<'INSTALLER'
#!/usr/bin/env bash
printf 'autospec-db installer ran\n' >> "$CURL_LOG"
INSTALLER
else
  cat <<'INSTALLER'
#!/usr/bin/env bash
printf 'autospec-db installer ran\n' >> "$CURL_LOG"
INSTALLER
fi
SH
    chmod +x "$BIN_DIR/curl"
    export PATH="$BIN_DIR:$PATH"
    export CURL_LOG="$LOG"
    write_db_prompt_runner
}

teardown() {
    rm -rf "$FAKE_HOME"
}

write_db_prompt_runner() {
    local script="$FAKE_HOME/run-db-prompt.sh"
    {
        printf '%s\n' '#!/usr/bin/env bash'
        printf '%s\n' 'set -eu'
        awk '/^while \[ \$# -gt 0 \]/{exit} { print }' "$INSTALL_SH" \
            | sed "s#^REPO_ROOT=.*#REPO_ROOT=\"$REPO_ROOT\"#"
        printf '%s\n' 'UPDATE="${UPDATE:-0}"'
        printf '%s\n' 'DRY_RUN="${DRY_RUN:-0}"'
        printf '%s\n' 'maybe_prompt_db_module'
    } > "$script"
    RUNNER="$script"
}

@test "install-db-module: force env installs without prompting" {
    export AUTOSPEC_INSTALL_DB=1
    run bash "$RUNNER"
    [ "$status" -eq 0 ]
    grep -q 'curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec-db/main/install.sh' "$LOG"
    grep -q 'autospec-db installer ran' "$LOG"
}

@test "install-db-module: AUTOSPEC_INSTALL_DB=0 skips even when installed" {
    mkdir -p "$HOME/.autospec/autospec-db"
    export AUTOSPEC_INSTALL_DB=0
    run bash "$RUNNER"
    [ "$status" -eq 0 ]
    [ ! -f "$LOG" ]
}

@test "install-db-module: installed module updates without prompting" {
    mkdir -p "$HOME/.autospec"
    touch "$HOME/.autospec/db.env"
    export UPDATE=1
    run bash "$RUNNER"
    [ "$status" -eq 0 ]
    grep -q 'autospec-db installer ran' "$LOG"
    [[ "$output" != *"Install the optional database telemetry module"* ]]
}

@test "install-db-module: absent update is silent" {
    export UPDATE=1
    run bash "$RUNNER"
    [ "$status" -eq 0 ]
    [ ! -f "$LOG" ]
    [[ "$output" != *"Install the optional database telemetry module"* ]]
}

@test "install-db-module: config-only db.conf prints finish hint and does not fetch" {
    mkdir -p "$HOME/.autospec"
    touch "$HOME/.autospec/db.conf"
    run bash "$RUNNER"
    [ "$status" -eq 0 ]
    [ ! -f "$LOG" ]
    [[ "$output" == *"autospec-db configuration is present"* ]]
    [[ "$output" == *"~/.autospec/db.env"* ]]
}

@test "install-db-module: no prompt under CI or non-TTY" {
    export CI=1
    run bash "$RUNNER"
    [ "$status" -eq 0 ]
    [ ! -f "$LOG" ]
    [[ "$output" != *"Install the optional database telemetry module"* ]]
}

@test "install-db-module: AUTOSPEC_NO_DB_PROMPT suppresses fresh prompt" {
    export AUTOSPEC_NO_DB_PROMPT=1
    run bash "$RUNNER"
    [ "$status" -eq 0 ]
    [ ! -f "$LOG" ]
    [[ "$output" != *"Install the optional database telemetry module"* ]]
}

@test "install-db-module: dry-run is quiet and does not fetch" {
    export DRY_RUN=1
    run bash "$RUNNER"
    [ "$status" -eq 0 ]
    [ ! -f "$LOG" ]
    [[ "$output" != *"Install the optional database telemetry module"* ]]
}

@test "install-db-module: installer failure warns but exits 0" {
    export AUTOSPEC_INSTALL_DB=1
    export CURL_FAIL=1
    run bash "$RUNNER"
    [ "$status" -eq 0 ]
    [[ "$output" == *"warn:"* ]]
    [[ "$output" == *"autospec-db installer failed; continuing"* ]]
}

@test "install-db-module: prompt text and validate hook are wired" {
    grep -qF 'Install the optional database telemetry module (autospec-db)? [y/N]' "$INSTALL_SH"
    grep -q 'maybe_prompt_db_module' "$INSTALL_SH"
    grep -q 'check_db_module_install' "$REPO_ROOT/scripts/validate.sh"
    grep -q 'tests/unit/install-db-module.bats' "$REPO_ROOT/scripts/validate.sh"
}

#!/usr/bin/env bats
# tests/unit/test_self_update_preflight.bats — exercise the startup preflight
# bash block (§7.1 scenario matrix) from skills/autospec/SKILL.md.
#
# Each test extracts the block via awk and runs it in a sandboxed $HOME so no
# real network calls ever leave the host. curl is shimmed where necessary.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"

setup() {
    # Sandboxed home — wiped after each test.
    export HOME
    HOME="$(mktemp -d)"

    # Extract the startup self-update bash block from the canonical source.
    # SKILL.md carries only the single-source block marker
    # (<!-- autospec-block:startup-self-update ... -->); awk over the raw
    # marker-only file would yield an empty block, so expand the markers first.
    SKILL="$REPO_ROOT/skills/autospec/SKILL.md"
    BLOCK="$(bash "$REPO_ROOT/scripts/expand-skill-blocks.sh" "$SKILL" \
        | awk '/^## Startup self-update/{f=1} f && /^```bash/{g=1; next} g && /^```/{g=0; f=0; next} g{print}')"

    # Shim directory — prepended to PATH inside each test that needs it.
    SHIMDIR="$(mktemp -d)"
    export SHIMDIR
    export BLOCK
    export REPO_ROOT
}

teardown() {
    rm -rf "$HOME"
    rm -rf "${SHIMDIR:-}"
}

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Run the block with PATH prepended by SHIMDIR (for shim tests).
_run_block_shimmed() {
    local h="$HOME"
    local sd="$SHIMDIR"
    run bash -c "export HOME='${h}'; PATH='${sd}:${PATH}'; ${BLOCK}"
}

# Run the block with the real PATH (for non-shim tests).
_run_block() {
    local h="$HOME"
    run bash -c "export HOME='${h}'; ${BLOCK}"
}

# ---------------------------------------------------------------------------
# Scenario 1 — First run: empty HOME, curl fails → fail-open, exit 0
# ---------------------------------------------------------------------------

@test "first run: curl absent/fails -> fail-open WARN, exit 0" {
    printf '#!/usr/bin/env bash\nexit 1\n' > "$SHIMDIR/curl"
    chmod +x "$SHIMDIR/curl"
    _run_block_shimmed
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "WARN:"
    [ ! -e "$HOME/.autospec/last-update-check" ]
}

# ---------------------------------------------------------------------------
# Scenario 2 — Within 24h rate-limit: last check was 1h ago → no network
# ---------------------------------------------------------------------------

@test "within 24h rate-limit: skips silently, exit 0, no WARN" {
    mkdir -p "$HOME/.autospec"
    # Write a timestamp 1 hour in the past.
    date -u -v-1H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d '1 hour ago' +'%Y-%m-%dT%H:%M:%SZ' \
        > "$HOME/.autospec/last-update-check"

    # Shim curl to fail loudly if invoked — should never be called.
    printf '#!/usr/bin/env bash\necho "UNEXPECTED curl call" >&2\nexit 1\n' > "$SHIMDIR/curl"
    chmod +x "$SHIMDIR/curl"
    _run_block_shimmed
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -q "UNEXPECTED"
}

# ---------------------------------------------------------------------------
# Scenario 3 — Past 24h: last check was 25h ago → network attempted
# ---------------------------------------------------------------------------

@test "past 24h rate-limit: network attempted, fail-open on failure" {
    mkdir -p "$HOME/.autospec"
    # Write a timestamp 25 hours in the past.
    old_check="$(date -u -v-25H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d '25 hours ago' +'%Y-%m-%dT%H:%M:%SZ' \
    )"
    printf '%s\n' "$old_check" > "$HOME/.autospec/last-update-check"

    # Shim curl to fail (simulating network down).
    printf '#!/usr/bin/env bash\nexit 1\n' > "$SHIMDIR/curl"
    chmod +x "$SHIMDIR/curl"
    _run_block_shimmed
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "WARN:"
    [ "$(cat "$HOME/.autospec/last-update-check")" = "$old_check" ]
}

# ---------------------------------------------------------------------------
# Scenario 4 — Opt-out env var: AUTOSPEC_NO_SELF_UPDATE=1 → silent skip
# ---------------------------------------------------------------------------

@test "AUTOSPEC_NO_SELF_UPDATE=1: silent skip, exit 0, no WARN" {
    # Shim curl to fail loudly if invoked — should never be called.
    printf '#!/usr/bin/env bash\necho "UNEXPECTED curl call" >&2\nexit 1\n' > "$SHIMDIR/curl"
    chmod +x "$SHIMDIR/curl"
    local h="$HOME"
    local sd="$SHIMDIR"
    run bash -c "export HOME='${h}'; export AUTOSPEC_NO_SELF_UPDATE=1; PATH='${sd}:${PATH}'; ${BLOCK}"
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -q "UNEXPECTED"
    ! echo "$output" | grep -q "WARN:"
}

# ---------------------------------------------------------------------------
# Scenario 5 — curl failure (network down): WARN logged, exit 0
# ---------------------------------------------------------------------------

@test "curl network failure: WARN logged, exit 0" {
    printf '#!/usr/bin/env bash\nexit 1\n' > "$SHIMDIR/curl"
    chmod +x "$SHIMDIR/curl"
    _run_block_shimmed
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "WARN:"
}

# ---------------------------------------------------------------------------
# Scenario 6 — install.sh non-zero: WARN logged, exit 0
# ---------------------------------------------------------------------------

@test "install.sh non-zero: WARN logged, exit 0" {
    mkdir -p "$HOME/.autospec"
    # Write a past timestamp so rate-limit gate passes.
    date -u -v-25H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d '25 hours ago' +'%Y-%m-%dT%H:%M:%SZ' \
        > "$HOME/.autospec/last-update-check"
    echo "oldsha1" > "$HOME/.autospec/installed-version"

    # Shim curl: version endpoint returns a different SHA; install URL returns a
    # script body that exits non-zero.
    cat > "$SHIMDIR/curl" << 'CURLSHIM'
#!/usr/bin/env bash
# Detect which URL is being requested via args
for arg in "$@"; do
    case "$arg" in
        *commits/main*)
            printf '{"sha":"newsha99"}\n'
            exit 0
            ;;
    esac
done
# Assume it's the install.sh fetch — emit a failing script.
printf '#!/usr/bin/env bash\nexit 42\n'
exit 0
CURLSHIM
    chmod +x "$SHIMDIR/curl"

    _run_block_shimmed
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "WARN:"
}

@test "install failure persists diagnostics without advancing successful check" {
    mkdir -p "$HOME/.autospec"
    echo "oldsha1" > "$HOME/.autospec/installed-version"

    cat > "$SHIMDIR/curl" << 'CURLSHIM'
#!/usr/bin/env bash
for arg in "$@"; do
    case "$arg" in
        *commits/main*) printf '{"sha":"newsha99"}\n'; exit 0 ;;
    esac
done
printf '#!/usr/bin/env bash\nprintf "compile error: cfg mismatch\\n" >&2\nexit 42\n'
CURLSHIM
    chmod +x "$SHIMDIR/curl"

    _run_block_shimmed
    [ "$status" -eq 0 ]
    block_output="$output"
    [ ! -e "$HOME/.autospec/last-update-check" ]
    [ "$(cat "$HOME/.autospec/installed-version")" = "oldsha1" ]
    [ "$(cat "$HOME/.autospec/remote-version")" = "newsha9" ]
    [ -s "$HOME/.autospec/self-update.log" ]
    [ "$(stat -f '%Lp' "$HOME/.autospec/self-update.log" 2>/dev/null || stat -c '%a' "$HOME/.autospec/self-update.log")" = "600" ]
    grep -q "compile error: cfg mismatch" "$HOME/.autospec/self-update.log"
    [ -s "$HOME/.autospec/last-update-failure.json" ]
    run jq -e '
        .timestamp | type == "string" and length > 0
    ' "$HOME/.autospec/last-update-failure.json"
    [ "$status" -eq 0 ]
    run jq -e '
        .remote_sha == "newsha9" and
        .installer_exit_code == 42 and
        (.output_tail | contains("compile error: cfg mismatch")) and
        (.log_path | endswith("/.autospec/self-update.log"))
    ' "$HOME/.autospec/last-update-failure.json"
    [ "$status" -eq 0 ]
    echo "$block_output" | grep -q "$HOME/.autospec/last-update-failure.json"
    echo "$block_output" | grep -q "$HOME/.autospec/self-update.log"
}

@test "failed install retries immediately and rotates a bounded diagnostic log" {
    mkdir -p "$HOME/.autospec"
    echo "oldsha1" > "$HOME/.autospec/installed-version"
    cat > "$SHIMDIR/curl" << 'CURLSHIM'
#!/usr/bin/env bash
for arg in "$@"; do
    case "$arg" in
        *commits/main*) printf '{"sha":"newsha99"}\n'; exit 0 ;;
    esac
done
printf '#!/usr/bin/env bash\nhead -c 100000 /dev/zero | tr "\\0" x\nprintf "\\nattempt-tail\\n" >&2\nexit 17\n'
CURLSHIM
    chmod +x "$SHIMDIR/curl"

    _run_block_shimmed
    [ "$status" -eq 0 ]
    first_size="$(wc -c < "$HOME/.autospec/self-update.log" | tr -d ' ')"
    [ "$first_size" -le 65536 ]
    _run_block_shimmed
    [ "$status" -eq 0 ]
    [ -s "$HOME/.autospec/self-update.log.1" ]
    [ "$(wc -c < "$HOME/.autospec/self-update.log" | tr -d ' ')" -le 65536 ]
    [ "$(wc -c < "$HOME/.autospec/self-update.log.1" | tr -d ' ')" -le 65536 ]
}

@test "installed-version publication failure never advances or reports success" {
    mkdir -p "$HOME/.autospec"
    echo "oldsha1" > "$HOME/.autospec/installed-version"
    cat > "$SHIMDIR/curl" << 'CURLSHIM'
#!/usr/bin/env bash
for arg in "$@"; do
    case "$arg" in
        *commits/main*) printf '{"sha":"newsha99"}\n'; exit 0 ;;
    esac
done
printf '#!/usr/bin/env bash\nprintf "install complete\\n"\nexit 0\n'
CURLSHIM
    cat > "$SHIMDIR/mv" << 'MVSHIM'
#!/usr/bin/env bash
for arg in "$@"; do target="$arg"; done
case "$target" in
    */installed-version) exit 73 ;;
    *) exec /bin/mv "$@" ;;
esac
MVSHIM
    chmod +x "$SHIMDIR/curl" "$SHIMDIR/mv"

    _run_block_shimmed
    [ "$status" -eq 0 ]
    [ "$(cat "$HOME/.autospec/installed-version")" = "oldsha1" ]
    [ ! -e "$HOME/.autospec/last-update-check" ]
    echo "$output" | grep -q "WARN: self-update state publication failed"
    ! echo "$output" | grep -q "\[autospec\] updated"
}

@test "success timestamp publication failure restores prior installed receipt" {
    mkdir -p "$HOME/.autospec"
    echo "oldsha1" > "$HOME/.autospec/installed-version"
    cat > "$SHIMDIR/curl" << 'CURLSHIM'
#!/usr/bin/env bash
for arg in "$@"; do
    case "$arg" in *commits/main*) printf '{"sha":"newsha99"}\n'; exit 0 ;; esac
done
printf '#!/usr/bin/env bash\nexit 0\n'
CURLSHIM
    cat > "$SHIMDIR/mv" << 'MVSHIM'
#!/usr/bin/env bash
for arg in "$@"; do target="$arg"; done
case "$target" in */last-update-check) exit 74 ;; *) exec /bin/mv "$@" ;; esac
MVSHIM
    chmod +x "$SHIMDIR/curl" "$SHIMDIR/mv"

    _run_block_shimmed
    [ "$status" -eq 0 ]
    [ "$(cat "$HOME/.autospec/installed-version")" = "oldsha1" ]
    [ ! -e "$HOME/.autospec/last-update-check" ]
    echo "$output" | grep -q "WARN: self-update state publication failed"
    ! echo "$output" | grep -q "\[autospec\] updated"
}

@test "success timestamp publication failure removes newly-created installed receipt" {
    mkdir -p "$HOME/.autospec"
    cat > "$SHIMDIR/curl" << 'CURLSHIM'
#!/usr/bin/env bash
for arg in "$@"; do
    case "$arg" in *commits/main*) printf '{"sha":"newsha99"}\n'; exit 0 ;; esac
done
printf '#!/usr/bin/env bash\nexit 0\n'
CURLSHIM
    cat > "$SHIMDIR/mv" << 'MVSHIM'
#!/usr/bin/env bash
for arg in "$@"; do target="$arg"; done
case "$target" in */last-update-check) exit 74 ;; *) exec /bin/mv "$@" ;; esac
MVSHIM
    chmod +x "$SHIMDIR/curl" "$SHIMDIR/mv"

    _run_block_shimmed
    [ "$status" -eq 0 ]
    [ ! -e "$HOME/.autospec/installed-version" ]
    [ ! -e "$HOME/.autospec/last-update-check" ]
    ! echo "$output" | grep -q "\[autospec\] updated"
}

# ---------------------------------------------------------------------------
# Scenario 6b — Installer target: suite bootstrap refreshes every skill
# ---------------------------------------------------------------------------

@test "startup self-update invokes suite bootstrap for all skills and harnesses" {
    mkdir -p "$HOME/.autospec"
    date -u -v-25H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d '25 hours ago' +'%Y-%m-%dT%H:%M:%SZ' \
        > "$HOME/.autospec/last-update-check"
    echo "oldsha1" > "$HOME/.autospec/installed-version"

    cat > "$SHIMDIR/curl" << 'CURLSHIM'
#!/usr/bin/env bash
for arg in "$@"; do
    case "$arg" in
        *commits/main*)
            printf '{"sha":"newsha99"}\n'
            exit 0
            ;;
        *raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh)
            printf '%s\n' "$arg" > "$HOME/install-url"
            printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$*" > "$HOME/install-args"\nexit 0\n'
            exit 0
            ;;
        *raw.githubusercontent.com/berlinguyinca/autospec/main/install.sh)
            printf '%s\n' "$arg" > "$HOME/unexpected-raw-installer"
            printf '#!/usr/bin/env bash\nexit 98\n'
            exit 0
            ;;
        *raw.githubusercontent.com/berlinguyinca/autospec/main/skills/*/install.sh)
            printf '%s\n' "$arg" > "$HOME/unexpected-skill-installer"
            printf '#!/usr/bin/env bash\nexit 99\n'
            exit 0
            ;;
    esac
done
exit 1
CURLSHIM
    chmod +x "$SHIMDIR/curl"

    _run_block_shimmed
    [ "$status" -eq 0 ]
    [ "$(cat "$HOME/install-url")" = "https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh" ]
    [ "$(cat "$HOME/install-args")" = "--skill all --harness all --update" ]
    [ ! -f "$HOME/unexpected-raw-installer" ]
    [ ! -f "$HOME/unexpected-skill-installer" ]
}

# ---------------------------------------------------------------------------
# Scenario 7 — Lock contention: lock dir already held → WARN, exit 0
# ---------------------------------------------------------------------------

@test "lock contention: WARN logged, exit 0" {
    mkdir -p "$HOME/.autospec/.update.lock.d"   # simulate lock already held
    # Shim curl to fail loudly if invoked — should never get past the lock.
    printf '#!/usr/bin/env bash\necho "UNEXPECTED curl call" >&2\nexit 1\n' > "$SHIMDIR/curl"
    chmod +x "$SHIMDIR/curl"
    _run_block_shimmed
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "WARN:"
    ! echo "$output" | grep -q "UNEXPECTED"
}

# ---------------------------------------------------------------------------
# Scenario 8 — Up-to-date no-op: installed == remote → no WARN, no banner
# ---------------------------------------------------------------------------

@test "up-to-date no-op: no WARN, no banner" {
    mkdir -p "$HOME/.autospec"
    # Write a timestamp 25h in the past so the rate-limit gate passes.
    date -u -v-25H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d '25 hours ago' +'%Y-%m-%dT%H:%M:%SZ' \
        > "$HOME/.autospec/last-update-check"
    echo "abc1234" > "$HOME/.autospec/installed-version"

    # Shim curl to return the same SHA as installed.
    cat > "$SHIMDIR/curl" << 'CURLSHIM'
#!/usr/bin/env bash
printf '{"sha":"abc1234"}\n'
exit 0
CURLSHIM
    chmod +x "$SHIMDIR/curl"

    _run_block_shimmed
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -q "WARN:"
    ! echo "$output" | grep -q "\[autospec\]"
}

@test "up-to-date self-update heals stale autonomous wrapper before remote no-op" {
    mkdir -p "$HOME/.autospec/bin" "$HOME/.autospec/scripts"
    date -u -v-25H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d '25 hours ago' +'%Y-%m-%dT%H:%M:%SZ' \
        > "$HOME/.autospec/last-update-check"
    echo "abc1234" > "$HOME/.autospec/installed-version"

    cat > "$HOME/.autospec/bin/autospec-autonomous-status" <<'STALE'
#!/usr/bin/env bash
set -eu
exec "/tmp/gone/autospec-autonomous.sh" status "$@"
STALE
    chmod +x "$HOME/.autospec/bin/autospec-autonomous-status"
    cat > "$HOME/.autospec/scripts/autospec-autonomous.sh" <<'LAUNCHER'
#!/usr/bin/env bash
printf '{"running":false,"args":"%s"}\n' "$*"
LAUNCHER
    chmod +x "$HOME/.autospec/scripts/autospec-autonomous.sh"

    cat > "$SHIMDIR/curl" << 'CURLSHIM'
#!/usr/bin/env bash
printf '{"sha":"abc1234"}\n'
exit 0
CURLSHIM
    chmod +x "$SHIMDIR/curl"

    _run_block_shimmed
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "heal_autonomous_operator_wrappers: healed"
    grep -qF 'exec "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-autonomous.sh" status "$@"' "$HOME/.autospec/bin/autospec-autonomous-status"
    run bash -c "HOME='${HOME}' '${HOME}/.autospec/bin/autospec-autonomous-status' --json"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '"running":false'
}

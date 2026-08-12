#!/usr/bin/env bats
# tests/autonomous/test_opencode_containment.bats — OpenCode implementer containment adapter.
#
# The executor bridge fails closed (executor_harness_uncontained) for mutating
# OpenCode work unless AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER resolves. This file
# pins the adapter's contract and its permission profile so the OpenCode
# implementer path does not rot: the adapter must (1) require a real OpenCode
# executable, (2) inject a deny-by-default permission profile that re-enables
# only read/edit/bash, (3) isolate config from ~/.config/opencode, and (4) exec
# the OpenCode binary with the remaining argv untouched.
#
# Mocking: PATH-shim `opencode` that records argv + env; no real harness, no network.

ADAPTER="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)/scripts/lib/opencode-containment-adapter.sh"

setup() {
    TMP="$(mktemp -d -t opencode-containment.XXXXXX)"
    mkdir -p "$TMP/bin"
    export PATH="$TMP/bin:$PATH"
}

teardown() {
    rm -rf "$TMP"
}

write_stub_opencode() {
    cat > "$TMP/bin/opencode" <<'EOF'
#!/usr/bin/env bash
# Record argv and the containment-relevant env so the test can assert on them.
{
    printf 'argv:'
    printf ' <%s>' "$@"
    printf '\n'
    printf 'OPENCODE_CONFIG_CONTENT=%s\n' "${OPENCODE_CONFIG_CONTENT:-<unset>}"
    printf 'OPENCODE_DISABLE_CLAUDE_CODE=%s\n' "${OPENCODE_DISABLE_CLAUDE_CODE:-<unset>}"
    printf 'OPENCODE_CONFIG_DIR=%s\n' "${OPENCODE_CONFIG_DIR:-<unset>}"
} > "${RECORD_FILE:?}"
EOF
    chmod +x "$TMP/bin/opencode"
}

@test "adapter refuses to run without an executable OpenCode" {
    run bash "$ADAPTER"
    [ "$status" -ne 0 ]
    [[ "$output" == *"usage"* ]]
}

@test "adapter fails when the OpenCode executable is missing" {
    run bash "$ADAPTER" "$TMP/bin/definitely-not-opencode" --pure run "prompt"
    [ "$status" -ne 0 ]
}

@test "adapter injects the implementer permission profile and isolates config" {
    export RECORD_FILE="$TMP/record.txt"
    write_stub_opencode

    run bash "$ADAPTER" "$TMP/bin/opencode" --pure run "implement issue 42"
    [ "$status" -eq 0 ]

    # argv must be passed through untouched after the executable.
    grep -q "<--pure> <run> <implement issue 42>" "$RECORD_FILE"

    # The permission profile must be deny-by-default with only read/edit/bash re-enabled.
    local cfg
    cfg="$(sed -n 's/^OPENCODE_CONFIG_CONTENT=//p' "$RECORD_FILE")"
    [[ "$cfg" == *'"*":"deny"'* ]]
    [[ "$cfg" == *'"edit":"allow"'* ]]
    [[ "$cfg" == *'"bash":"allow"'* ]]
    [[ "$cfg" == *'"external_directory":"deny"'* ]]
    [[ "$cfg" == *'"webfetch":"deny"'* ]]
    [[ "$cfg" == *'"websearch":"deny"'* ]]
    [[ "$cfg" == *'"task":"deny"'* ]]
    [[ "$cfg" == *'"skill":"deny"'* ]]

    # Host config must never be inherited and config must be isolated.
    grep -q "OPENCODE_DISABLE_CLAUDE_CODE=1" "$RECORD_FILE"
    local cfg_dir
    cfg_dir="$(sed -n 's/^OPENCODE_CONFIG_DIR=//p' "$RECORD_FILE")"
    [ -n "$cfg_dir" ]
    [ "$cfg_dir" != "$HOME/.config/opencode" ]
}

@test "adapter overrides a pre-existing OPENCODE_CONFIG_DIR with its private dir" {
    export RECORD_FILE="$TMP/record.txt"
    write_stub_opencode
    export OPENCODE_CONFIG_DIR="$HOME/.config/opencode"

    run bash "$ADAPTER" "$TMP/bin/opencode" --pure run "prompt"
    [ "$status" -eq 0 ]

    # The adapter must not leak the operator's real config dir into the run.
    grep -q "OPENCODE_DISABLE_CLAUDE_CODE=1" "$RECORD_FILE"
    local cfg_dir
    cfg_dir="$(sed -n 's/^OPENCODE_CONFIG_DIR=//p' "$RECORD_FILE")"
    [ -n "$cfg_dir" ]
    [ "$cfg_dir" != "$HOME/.config/opencode" ]
}

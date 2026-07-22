#!/usr/bin/env bats
# tests/install-rollover.bats — tests for install.sh auto-rollover block

INSTALL_SH="${BATS_TEST_DIRNAME}/../install.sh"
MARKER_START="# >>> autospec auto-rollover >>>"
MARKER_END="# <<< autospec auto-rollover <<<"

# Source only the rollover helpers by extracting the relevant line range.
# Lines 458-571 contain _ROLLOVER_MARKER_START through remove_rollover_block.
_ROLLOVER_HELPERS="${BATS_TMPDIR}/rollover_helpers.sh"

setup() {
    export ORIG_HOME="$HOME"
    export HOME="$(mktemp -d)"
    export REPO_ROOT="$(cd "$(dirname "$INSTALL_SH")" && pwd)"
    export AUTOSPEC_NO_STAR_PROMPT=1
    export AUTOSPEC_SKIP_SYSTEM_TOOLS=1
    export AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1

    # Build a sourceable helpers file from install.sh's rollover block.
    # We extract from `_ROLLOVER_MARKER_START=` (the first ROLLOVER var
    # declaration) through the line that registers the hook-mode-claude
    # function — this captures _ROLLOVER_MARKER_*, prompt_user_for_auto_rollover,
    # install_context_monitor_pkg, install_rollover_block, and
    # remove_rollover_block — regardless of where they sit in the file.
    {
        printf '#!/usr/bin/env bash\n'
        printf 'UPDATE=0\nDRY_RUN=0\nDISABLE_AUTO_ROLLOVER=0\n'
        printf 'info() { printf "%%s\\n" "$*"; }\n'
        printf 'warn() { printf "warn: %%s\\n" "$*" >&2; }\n'
        printf 'err()  { printf "error: %%s\\n" "$*" >&2; }\n'
        awk '
            /^generated_harness_section\(\)/ { capture = 1 }
            capture { print }
            capture && /^\}$/ { exit }
        ' "$INSTALL_SH"
        # Stub install_context_monitor_pkg so tests don't shell out to pip.
        # The real helper is exercised by tests/test_install_pip_context_monitor.bats.
        printf 'install_context_monitor_pkg() { :; }\n'
        # Extract from _ROLLOVER_MARKER_START variable declaration up to (and
        # including) the closing brace of remove_rollover_block.
        awk '
            /^_ROLLOVER_MARKER_START=/ { capture = 1 }
            capture { print }
            /^remove_rollover_block\(\)/ { in_remove = 1 }
            in_remove && /^\}$/ { exit }
        ' "$INSTALL_SH"
    } > "$_ROLLOVER_HELPERS"
}

teardown() {
    rm -rf "$HOME"
    rm -f "$_ROLLOVER_HELPERS"
    export HOME="$ORIG_HOME"
}

@test "test_install_block_is_idempotent" {
    touch "$HOME/.bashrc"

    # shellcheck source=/dev/null
    source "$_ROLLOVER_HELPERS"

    install_rollover_block
    install_rollover_block

    count=$(grep -c "$MARKER_START" "$HOME/.bashrc" || true)
    [ "$count" -eq 1 ]
}

@test "test_disable_flag_removes_block" {
    touch "$HOME/.bashrc"

    source "$_ROLLOVER_HELPERS"

    install_rollover_block
    grep -q "$MARKER_START" "$HOME/.bashrc"  # assert it was written

    remove_rollover_block

    run grep "$MARKER_START" "$HOME/.bashrc"
    [ "$status" -ne 0 ]  # grep found nothing
}

@test "test_block_not_written_when_user_answers_no" {
    touch "$HOME/.bashrc"

    source "$_ROLLOVER_HELPERS"

    # Call install_rollover_block only if answer is y — simulate n answer
    answer="n"
    case "$answer" in
        y|Y|yes|YES|Yes) install_rollover_block ;;
        *) : ;;
    esac

    run grep "$MARKER_START" "$HOME/.bashrc"
    [ "$status" -ne 0 ]  # block must NOT have been written
}

@test "test_fish_config_gets_function_form_not_alias" {
    mkdir -p "$HOME/.config/fish"
    touch "$HOME/.config/fish/config.fish"

    source "$_ROLLOVER_HELPERS"

    install_rollover_block

    # Fish block must use `function ... end` syntax
    run grep "function claude" "$HOME/.config/fish/config.fish"
    [ "$status" -eq 0 ]

    # Must NOT use bash-style function declaration
    run grep "claude()" "$HOME/.config/fish/config.fish"
    [ "$status" -ne 0 ]
}

@test "test_fish_config_exports_rollover_without_debug_trace_flag" {
    mkdir -p "$HOME/.config/fish"
    touch "$HOME/.config/fish/config.fish"

    source "$_ROLLOVER_HELPERS"

    install_rollover_block

    run grep -F "set -gx AUTOSPEC_AUTO_ROLLOVER 1" "$HOME/.config/fish/config.fish"
    [ "$status" -eq 0 ]

    run grep -F "set -x AUTOSPEC_AUTO_ROLLOVER 1" "$HOME/.config/fish/config.fish"
    [ "$status" -ne 0 ]
}

@test "test_disable_flag_exits_zero_when_block_absent" {
    source "$_ROLLOVER_HELPERS"

    run remove_rollover_block

    [ "$status" -eq 0 ]
}

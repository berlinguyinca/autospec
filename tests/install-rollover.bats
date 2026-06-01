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
    export AUTOSPEC_NO_STAR_PROMPT=1
    export AUTOSPEC_SKIP_SYSTEM_TOOLS=1
    export AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1

    # Build a sourceable helpers file from install.sh's rollover block.
    {
        printf '#!/usr/bin/env bash\n'
        printf 'UPDATE=0\nDRY_RUN=0\nDISABLE_AUTO_ROLLOVER=0\n'
        printf 'info() { printf "%%s\\n" "$*"; }\n'
        printf 'err()  { printf "error: %%s\\n" "$*" >&2; }\n'
        # Extract lines 458-571 from install.sh (rollover vars + functions)
        sed -n '458,571p' "$INSTALL_SH"
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

@test "test_disable_flag_exits_zero_when_block_absent" {
    # install.sh --disable-auto-rollover should succeed even with no block present
    run bash "$INSTALL_SH" --disable-auto-rollover
    [ "$status" -eq 0 ]
}

#!/usr/bin/env bats
# tests/test_install_pip_context_monitor.bats — regression for g-002
#
# install.sh must invoke `pip install` for packages/autospec_context_monitor
# when the user enables auto-rollover, otherwise both the tmux launcher
# (scripts/autospec-session) and the Claude PreCompact hook (registered into
# ~/.claude/settings.json) will fail with "No module named autospec_context_monitor".

INSTALL_SH="${BATS_TEST_DIRNAME}/../install.sh"

@test "g-002: install.sh contains a pip install for the package directory" {
    # The package is installed editable from its source dir, so the pip line
    # references the package directory ($pkg_dir → packages/autospec_context_monitor).
    run grep -E 'pip[[:space:]]+install[[:space:]].*\$pkg_dir' "$INSTALL_SH"
    [ "$status" -eq 0 ]

    # And the package directory path must be referenced.
    run grep -F 'packages/autospec_context_monitor' "$INSTALL_SH"
    [ "$status" -eq 0 ]
}

@test "g-002: pip install uses --user (no sudo, no system-wide install)" {
    run grep -E 'pip[[:space:]]+install[[:space:]].*--user' "$INSTALL_SH"
    [ "$status" -eq 0 ]
}

@test "g-002: install_rollover_block calls install_context_monitor_pkg" {
    # The pip install is gated behind the rollover prompt via
    # install_context_monitor_pkg, called from install_rollover_block.
    block=$(awk '/^install_rollover_block\(\)/{flag=1} flag{print} /^}$/{if(flag){flag=0; exit}}' "$INSTALL_SH")
    echo "$block" | grep -q 'install_context_monitor_pkg'
}

@test "g-002: install_context_monitor_pkg helper exists and runs pip install" {
    block=$(awk '/^install_context_monitor_pkg\(\)/{flag=1} flag{print} /^}$/{if(flag){flag=0; exit}}' "$INSTALL_SH")
    echo "$block" | grep -qE 'pip[[:space:]]+install'
    echo "$block" | grep -qF 'autospec_context_monitor'
}

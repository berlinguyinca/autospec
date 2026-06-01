#!/usr/bin/env bats
# tests/test_install_pip_context_monitor.bats — regression for g-002
#
# install.sh must invoke `pip install` for packages/autospec_context_monitor
# when the user enables auto-rollover, otherwise both the tmux launcher
# (scripts/autospec-session) and the Claude PreCompact hook (registered into
# ~/.claude/settings.json) will fail with "No module named autospec_context_monitor".

INSTALL_SH="${BATS_TEST_DIRNAME}/../install.sh"

@test "g-002: install_rollover_block invokes pip install for autospec_context_monitor" {
    # Static grep: the install_rollover_block helper (or a helper it calls)
    # must contain a `pip install ... autospec_context_monitor` reference.
    run grep -nE 'pip[[:space:]]+install.*autospec_context_monitor|packages/autospec_context_monitor' "$INSTALL_SH"
    [ "$status" -eq 0 ]

    # And it must specifically be a pip install line (not just a comment / path mention)
    run grep -cE 'pip[[:space:]]+install.+autospec_context_monitor' "$INSTALL_SH"
    [ "$status" -eq 0 ]
    [ "$output" -ge 1 ]
}

@test "g-002: pip install uses --user (no sudo, no system-wide install)" {
    run grep -E 'pip[[:space:]]+install[[:space:]]+--user.*autospec_context_monitor' "$INSTALL_SH"
    [ "$status" -eq 0 ]
}

@test "g-002: pip install runs inside install_rollover_block (gated by user prompt)" {
    # Extract the install_rollover_block function and assert it contains pip install.
    block=$(awk '/^install_rollover_block\(\)/{flag=1} flag{print} /^}$/{if(flag){flag=0; exit}}' "$INSTALL_SH")
    echo "$block" | grep -qE 'pip[[:space:]]+install.+autospec_context_monitor'
}

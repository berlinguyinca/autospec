#!/usr/bin/env bats
# tests/unit/test_stop_inline_subagent_regex.bats — asserts the regex
# `^\s*stop(\s+--\w+)*\s*$` matches the spec §9.3 accept list and rejects
# the §9.3 reject list, using bash [[ =~ ]] with nocasematch.
#
# Spec ref: docs/specs/2026-05-01-autospec-stop-mechanism-design.md §9.3

# The regex as defined in the spec
RX='^[[:space:]]*stop([[:space:]]+--[[:alnum:]_]+)*[[:space:]]*$'

# Helper: test a string against the regex with nocasematch
matches() {
    local s="$1"
    bash -c "shopt -s nocasematch; [[ \"\$1\" =~ ${RX} ]]" -- "$s"
}

# ---- ACCEPT cases (spec §9.3) -----------------------------------------------

@test "accepts: 'stop'" {
    matches "stop"
}

@test "accepts: 'stop --graceful'" {
    matches "stop --graceful"
}

@test "accepts: 'stop --immediate'" {
    matches "stop --immediate"
}

@test "accepts: 'stop --status'" {
    matches "stop --status"
}

@test "accepts: 'stop --resume'" {
    matches "stop --resume"
}

@test "accepts: '  stop  --graceful  ' (whitespace padding)" {
    matches "  stop  --graceful  "
}

# ---- REJECT cases (spec §9.3) -----------------------------------------------

@test "rejects: 'stop me now'" {
    ! matches "stop me now"
}

@test "rejects: 'stopover'" {
    ! matches "stopover"
}

@test "rejects: 'stop ; rm -rf'" {
    ! matches "stop ; rm -rf"
}

@test "rejects: 'stop\n--graceful' (literal backslash-n)" {
    ! matches 'stop\n--graceful'
}

@test "rejects: 'STOP HERE'" {
    ! matches "STOP HERE"
}

@test "rejects: 'stoppppp'" {
    ! matches "stoppppp"
}

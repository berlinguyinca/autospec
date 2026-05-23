#!/usr/bin/env bash
# tests/fixtures/bash-mutate/sample.sh — fixture bash file for mutation testing.
set -eu

check_equal() {
    local val="$1"
    if [ "$val" == "expected" ]; then
        echo "match"
    fi
}

assert_output() {
    local out="$1"
    [ "$out" == "match" ]
}

run_check() {
    run check_equal "expected"
    grep -q "match" <<< "$output"
    [ "$status" -eq 0 ]
}

#!/usr/bin/env bash
# istanbul.sh — normalize Istanbul lcov output to canonical lcov on stdout.
#
# Usage: collect <lcov_path>
#   <lcov_path>: path to lcov.info produced by Istanbul/nyc/jest --coverage
#
# Outputs canonical lcov on stdout.
# Exit 0 = ok, 1 = fatal (file not found / unreadable).

set -eu

collect() {
    local lcov_path="${1:-}"
    if [ -z "$lcov_path" ]; then
        printf 'istanbul: fatal: lcov_path argument required\n' >&2
        exit 1
    fi
    if [ ! -f "$lcov_path" ]; then
        printf 'istanbul: fatal: lcov file not found: %s\n' "$lcov_path" >&2
        exit 1
    fi
    # Istanbul lcov is already canonical lcov format — pass through.
    cat "$lcov_path"
}

collect "${1:-}"

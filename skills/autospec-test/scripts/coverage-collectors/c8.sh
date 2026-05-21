#!/usr/bin/env bash
# c8.sh — collect coverage from c8 and emit canonical lcov on stdout.
#
# Usage: collect <c8_output_dir>
#   <c8_output_dir>: directory where c8 writes coverage reports
#   If c8 hasn't been run yet, this script runs: c8 report --reporter=lcovonly
#
# Exit 0 = ok, 1 = fatal.

set -eu

collect() {
    local c8_dir="${1:-coverage}"

    # If a pre-generated lcov.info exists, use it directly.
    if [ -f "$c8_dir/lcov.info" ]; then
        cat "$c8_dir/lcov.info"
        return 0
    fi

    # Try to generate via c8 CLI
    if command -v c8 >/dev/null 2>&1; then
        if ! c8 report --reporter=lcovonly --reports-dir "$c8_dir" 2>/dev/null; then
            printf 'c8: fatal: c8 report failed\n' >&2
            exit 1
        fi
        if [ -f "$c8_dir/lcov.info" ]; then
            cat "$c8_dir/lcov.info"
            return 0
        fi
    fi

    printf 'c8: fatal: no lcov.info found in %s and c8 CLI unavailable\n' "$c8_dir" >&2
    exit 1
}

collect "${1:-coverage}"

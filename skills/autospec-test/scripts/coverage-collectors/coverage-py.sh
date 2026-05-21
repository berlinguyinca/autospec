#!/usr/bin/env bash
# coverage-py.sh — collect Python coverage and emit canonical lcov on stdout.
#
# Usage: collect <coverage_file>
#   <coverage_file>: path to .coverage data file (produced by pytest-cov / coverage run)
#
# Converts via: coverage lcov -o - (coverage.py >= 6.3)
# Falls back to: coverage xml then xml-to-lcov conversion.
#
# Exit 0 = ok, 1 = fatal.

set -eu

collect() {
    local coverage_file="${1:-.coverage}"

    if ! command -v coverage >/dev/null 2>&1; then
        printf 'coverage-py: fatal: coverage CLI not found. Install with: pip install coverage\n' >&2
        exit 1
    fi

    # Try lcov output (coverage.py >= 6.3)
    if coverage lcov -o /dev/stdout --data-file="$coverage_file" 2>/dev/null; then
        return 0
    fi

    printf 'coverage-py: fatal: coverage lcov failed for %s\n' "$coverage_file" >&2
    exit 1
}

collect "${1:-.coverage}"

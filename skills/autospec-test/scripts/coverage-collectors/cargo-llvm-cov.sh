#!/usr/bin/env bash
# cargo-llvm-cov.sh — collect Rust coverage via cargo-llvm-cov and emit lcov.
#
# Usage: collect [<project_dir>]
#   <project_dir>: root of the Rust project (default: current dir)
#
# Requires: cargo install cargo-llvm-cov
# Exit 0 = ok, 1 = fatal.

set -eu

collect() {
    local project_dir="${1:-.}"

    if ! command -v cargo >/dev/null 2>&1; then
        printf 'cargo-llvm-cov: fatal: cargo not found\n' >&2
        exit 1
    fi

    if ! cargo llvm-cov --version >/dev/null 2>&1; then
        printf 'cargo-llvm-cov: fatal: cargo-llvm-cov not installed. Install with: cargo install cargo-llvm-cov\n' >&2
        exit 1
    fi

    (cd "$project_dir" && cargo llvm-cov --lcov 2>/dev/null)
}

collect "${1:-.}"

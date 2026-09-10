#!/usr/bin/env bash
# scripts/benchmark-validate-matrix.sh — validate a Qwen3.8 benchmark matrix
# against the provider-neutral contract (issue #3328).
#
# Usage: scripts/benchmark-validate-matrix.sh <matrix.json> [--json]
#
# Runs only supported local cells (Q3-Q8 quantizations, supported runtimes,
# supported nodes) and preserves the skip reason for every skipped cell.
# A candidate wins only when success: true; the final report compares the
# winner's median successful issue time to the baseline.
set -euo pipefail

if [ "$#" -lt 1 ]; then
    echo "usage: $(basename "$0") <matrix.json> [--json]" >&2
    exit 2
fi

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
bin="${CARGO_TARGET_DIR:-$repo_root/target}/debug/autospec"

if [ ! -x "$bin" ]; then
    cargo build --quiet -p autospec-cli --manifest-path "$repo_root/Cargo.toml"
fi

exec "$bin" benchmark validate-matrix "$@"

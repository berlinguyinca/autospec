#!/usr/bin/env bash
# scripts/autospec-constitutional-gap-v1.sh — rule-based constitutional gap report.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
exec python3 "$SCRIPT_DIR/autospec-constitution-rules.py" gap "$@"

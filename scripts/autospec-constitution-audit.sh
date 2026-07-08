#!/usr/bin/env bash
# scripts/autospec-constitution-audit.sh — run the local Constitution rule audit.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
exec python3 "$SCRIPT_DIR/autospec-constitution-rules.py" audit "$@"

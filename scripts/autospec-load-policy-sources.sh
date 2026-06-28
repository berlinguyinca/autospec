#!/usr/bin/env bash
# scripts/autospec-load-policy-sources.sh — discover structured policy source files.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
exec python3 "$SCRIPT_DIR/autospec-constitution-rules.py" load "$@"

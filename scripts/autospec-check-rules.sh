#!/usr/bin/env bash
# scripts/autospec-check-rules.sh — evaluate effective rules against Digital Twin metadata.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
exec python3 "$SCRIPT_DIR/autospec-constitution-rules.py" check "$@"

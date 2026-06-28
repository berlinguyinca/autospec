#!/usr/bin/env bash
# scripts/autospec-extract-constitution-rules.sh — extract Constitution/Baseline rules.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
exec python3 "$SCRIPT_DIR/autospec-constitution-rules.py" extract "$@"

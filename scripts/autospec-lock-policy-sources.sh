#!/usr/bin/env bash
# scripts/autospec-lock-policy-sources.sh — lock configured policy inputs.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
exec python3 "$SCRIPT_DIR/autospec-constitution-rules.py" lock-sources "$@"

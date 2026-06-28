#!/usr/bin/env bash
# scripts/autospec-validate-policy-sources.sh — validate configured policy sources.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
exec python3 "$SCRIPT_DIR/autospec-constitution-rules.py" validate-sources "$@"

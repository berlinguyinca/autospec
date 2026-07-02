#!/usr/bin/env bash
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
exec python3 "$SCRIPT_DIR/autospec-baseline-v25.py" --command v25-status "$@"

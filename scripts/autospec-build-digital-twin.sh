#!/usr/bin/env bash
# scripts/autospec-build-digital-twin.sh — build local Digital Twin metadata.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
exec python3 "$SCRIPT_DIR/autospec-digital-twin.py" build "$@"

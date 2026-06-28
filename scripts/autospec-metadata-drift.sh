#!/usr/bin/env bash
# scripts/autospec-metadata-drift.sh — report stale or missing local metadata.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
exec python3 "$SCRIPT_DIR/autospec-digital-twin.py" drift "$@"

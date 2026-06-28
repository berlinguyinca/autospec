#!/usr/bin/env bash
# scripts/autospec-policy-compatibility.sh — report policy features unsupported by this engine.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
exec python3 "$SCRIPT_DIR/autospec-constitution-rules.py" compatibility "$@"

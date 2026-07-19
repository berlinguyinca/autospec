#!/usr/bin/env bash
# scripts/autospec-impact-analysis.sh — heuristic impact analysis from the local knowledge graph.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
exec python3 "$SCRIPT_DIR/autospec-digital-twin.py" impact "$@"

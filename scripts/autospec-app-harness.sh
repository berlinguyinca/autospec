#!/usr/bin/env bash
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
args=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --command) args+=(--command-line "$2"); shift 2 ;;
    *) args+=("$1"); shift ;;
  esac
done
exec python3 "$SCRIPT_DIR/autospec-evidence-v1-lib.py" --command app-harness "${args[@]}"

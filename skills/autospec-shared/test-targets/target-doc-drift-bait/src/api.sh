#!/usr/bin/env bash
# api.sh — simple HTTP API stub
# Options: --port <N>  --timeout <secs>  --verbose
set -euo pipefail

PORT=8080
TIMEOUT=30
VERBOSE=0

while [ $# -gt 0 ]; do
    case "$1" in
        --port)    PORT="$2";    shift 2 ;;
        --timeout) TIMEOUT="$2"; shift 2 ;;
        --verbose) VERBOSE=1;    shift   ;;
        *) echo "unknown: $1" >&2; exit 2 ;;
    esac
done

echo "listening on port $PORT (timeout=${TIMEOUT}s, verbose=${VERBOSE})"

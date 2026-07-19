#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/autospec-v61-v70.py --repo-root . --version 64 --action status "$@"

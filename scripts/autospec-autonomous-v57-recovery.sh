#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/autospec-baseline-v25.py --repo-root . --command v57-recovery "$@"

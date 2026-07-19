#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "$#" -eq 0 ]; then
  python3 scripts/autospec-baseline-v25.py --repo-root . --command baseline-validation >/dev/null
  for version in $(seq 26 60); do
    python3 scripts/autospec-baseline-v25.py --repo-root . --command "v${version}-supervisor" --prepare-only >/dev/null
  done
fi
python3 scripts/autospec-baseline-v25.py --repo-root . --command v60-status "$@"

#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/autospec-baseline-validation.sh --repo-root . >/dev/null
echo "complete_autonomy_gate: dry_run_pass"
echo "forbidden operations: false"

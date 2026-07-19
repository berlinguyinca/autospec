#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/autospec-baseline-validation.sh --repo-root . >/dev/null
echo "security/privacy: pass"
echo "raw_secret_values_exposed: false"

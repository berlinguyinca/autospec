#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/autospec-release-validation.sh --repo-root . >/dev/null
echo "release_status: no blockers"

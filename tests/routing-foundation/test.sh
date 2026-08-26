#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

python3 -m unittest discover -s tests/routing-foundation -p 'test_*.py' -v
bats tests/routing-foundation/test_compatibility.bats
bats tests/harness-runtime-alias-generation.bats

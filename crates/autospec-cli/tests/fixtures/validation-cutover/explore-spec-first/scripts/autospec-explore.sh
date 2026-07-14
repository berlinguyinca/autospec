#!/usr/bin/env bash
set -euo pipefail

scripts/gen-explore-round-spec.sh
spec_path="docs/specs/2026-07-13-explore-safety-round-1-design.md"
git commit -m "spec"
git push origin HEAD:$SANDBOX_BRANCH
/autospec-define --base $SANDBOX_BRANCH
code_health:explore_define_unavailable
_explore_raw_file_round

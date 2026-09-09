#!/usr/bin/env bash
# A tool that lives in the repository (not a scratch path) is fine to reuse —
# it already has history, a test, and an owner.
set -eu
bash scripts/tools/convert.sh wt-1
bash scripts/tools/convert.sh wt-2
bash scripts/tools/convert.sh wt-3
bash scripts/tools/convert.sh wt-4
bash scripts/tools/convert.sh wt-5

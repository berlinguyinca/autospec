#!/usr/bin/env bash
# Boundary: a scratch tool invoked exactly twice is NOT "more than twice".
set -eu
bash /tmp/gw-as-3977-22910005/tools/boundary.sh wt-1
bash /tmp/gw-as-3977-22910005/tools/boundary.sh wt-2

#!/usr/bin/env bash
# ... and once here. Counted across the session (2 + 1 = 3 > 2) the split tool
# is a promotion candidate even though no single agent used it more than twice.
set -eu
bash /tmp/gw-as-3977-22910009/tools/split.sh part-c

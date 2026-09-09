#!/usr/bin/env bash
# An ephemeral mktemp-style helper (XXXXXX template) is a per-run temp file,
# not a promotion candidate — exempt even when used repeatedly.
set -eu
bash /tmp/gw-as-3977-22910006/helper.XXXXXX.sh a
bash /tmp/gw-as-3977-22910006/helper.XXXXXX.sh b
bash /tmp/gw-as-3977-22910006/helper.XXXXXX.sh c
bash /tmp/gw-as-3977-22910006/helper.XXXXXX.sh d

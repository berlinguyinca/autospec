#!/usr/bin/env bash
# One of two agents in the same session. The split tool is used twice here ...
set -eu
bash /tmp/gw-as-3977-22910009/tools/split.sh part-a
bash /tmp/gw-as-3977-22910009/tools/split.sh part-b

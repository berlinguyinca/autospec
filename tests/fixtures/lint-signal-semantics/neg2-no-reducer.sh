#!/usr/bin/env bash
# Negative fixture: stderr discarded but no counting reducer follows; the
# output is not being turned into a threshold-checked number.
set -eu
out=$(some_cmd 2>/dev/null | tail -5)
echo "$out"

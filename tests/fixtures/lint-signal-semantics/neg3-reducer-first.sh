#!/usr/bin/env bash
# Negative fixture: the count precedes the discard; the reducer is not fed by
# the swallowed stream.
set -eu
n=$(grep -c pattern file 2>/dev/null || printf 0)
echo "$n"

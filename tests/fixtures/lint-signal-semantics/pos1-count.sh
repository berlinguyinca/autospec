#!/usr/bin/env bash
# Positive fixture: stderr swallowed into a count; failure and empty are both 0.
set -eu
count=$(some_cmd 2>/dev/null | wc -l)
echo "$count"

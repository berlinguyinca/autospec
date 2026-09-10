#!/usr/bin/env bash
# Negative fixture: no stderr discard; errors stay visible, the count is honest.
set -eu
count=$(some_cmd | wc -l)
echo "$count"

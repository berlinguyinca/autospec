#!/usr/bin/env bash
# Positive fixture: spaced redirect variant is still a swallow.
set -eu
n=$(head -c 100 file 2> /dev/null | wc -c)
echo "$n"

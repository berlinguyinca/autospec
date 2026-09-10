#!/usr/bin/env bash
# Positive fixture: grep -c after a swallowed stderr; a broken rg reports 0.
set -eu
n=$(rg -n pattern dir 2>/dev/null | grep -cF x)
echo "$n"

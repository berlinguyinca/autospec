#!/usr/bin/env bash
# Negative: counter loop — the done line has no redirect, so it is not
# stdin-fed. The `gh` in the body is not a finding.
set -eu
i=1
while [ "$i" -le 3 ]; do
    gh api rate_limit
    i=$((i + 1))
done

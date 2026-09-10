#!/usr/bin/env bash
# Negative: the outer loop is FD-0, but the `gh` runs in the *nested* loop's
# body, which is excluded from the outer loop's direct body. The inner loop has
# no redirect, so neither loop is a finding.
set -eu
GROUPS=/tmp/groups.txt
while IFS= read -r group; do
    for issue in a b c; do
        gh issue view "$issue"
    done
done < "$GROUPS"

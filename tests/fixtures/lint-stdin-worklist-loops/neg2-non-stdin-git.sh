#!/usr/bin/env bash
# Negative: `git log` reads the object database, not stdin. NOT a finding.
set -eu
REFS=/tmp/refs.txt
while IFS= read -r ref; do
    git log -1 "$ref"
done < "$REFS"

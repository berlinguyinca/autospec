#!/usr/bin/env bash
# Negative: the fix. The worklist is read on a dedicated FD (`done 3<`), so the
# loop body's `gh` no longer eats it. NOT a finding.
set -eu
WORKLIST=/tmp/worklist.txt
while IFS= read -r issue <&3; do
    gh issue view "$issue" --repo "$REPO"
done 3< "$WORKLIST"

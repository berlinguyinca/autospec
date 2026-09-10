#!/usr/bin/env bash
# Positive: a while loop reads its worklist from stdin (`done < file`) and runs
# a literal `gh` in the body — the canonical #3742 bug.
set -eu
REPO=berlinguyinca/autospec
WORKLIST=/tmp/worklist.txt
while IFS= read -r issue; do
    gh issue view "$issue" --repo "$REPO"
done < "$WORKLIST"

#!/usr/bin/env bash
# Positive: `$(gh ...)` inherits the loop's stdin and eats the worklist. The
# `gh` is buried in a command substitution (via a GH_ variable alias) that the
# token scanner cannot see, so the `$( ... )` regex catches it.
set -eu
GH_BIN=gh
TELE=/tmp/tele.txt
while IFS= read -r need || [ -n "$need" ]; do
    blob="$("$GH_BIN" issue view "$need" --repo "$REPO" 2>/dev/null)"
    echo "$blob"
done < "$TELE"

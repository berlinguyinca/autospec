#!/usr/bin/env bash
# Positive: `git apply` reads a patch from stdin; in an FD-0 worklist loop it
# consumes the worklist. Only stdin-consuming git subcommands are findings.
set -eu
PATCHES=/tmp/patches.txt
while IFS= read -r patch; do
    git apply "$patch"
done < "$PATCHES"

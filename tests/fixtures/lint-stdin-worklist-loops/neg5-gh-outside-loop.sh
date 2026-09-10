#!/usr/bin/env bash
# Negative: a `gh` outside any loop, plus a clean FD-0 loop whose body has no
# stdin-consuming command. NOT a finding.
set -eu
gh auth status
NAMES=/tmp/names.txt
while IFS= read -r name; do
    echo "$name"
done < "$NAMES"

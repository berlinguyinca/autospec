#!/usr/bin/env bash
# Negative fixture: the waiver marker is bare (no reason), so it is rejected
# and the line below stays a finding.
set -eu
# linter:allow-SIGNAL_SEMANTICS
count=$(some_cmd 2>/dev/null | wc -l)
echo "$count"

#!/usr/bin/env bash
# Negative fixture: waiver on the line immediately above (reason mandatory).
set -eu
# linter:allow-SIGNAL_SEMANTICS zero here means "nothing to do"; the command cannot fail in this context
count=$(some_cmd 2>/dev/null | wc -l)
echo "$count"

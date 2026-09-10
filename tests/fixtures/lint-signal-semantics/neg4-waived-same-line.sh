#!/usr/bin/env bash
# Negative fixture: same-line waiver with a reason silences the finding.
set -eu
count=$(some_cmd 2>/dev/null | wc -l) # linter:allow-SIGNAL_SEMANTICS count is advisory here, failures surface via the exit code below
[ -n "$(some_cmd)" ]

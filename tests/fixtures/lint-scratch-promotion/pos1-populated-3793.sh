#!/usr/bin/env bash
# Supervision session command log — issue #3793 populated case.
# A conversion tool written into a job-scoped temp dir is used across three
# worktrees. Used more than twice => it is a tool, not throwaway.
set -eu

# the conversion tool, invoked three times (one per worktree):
bash /tmp/gw-as-3977-22910004/tools/convert.sh wt-1
bash /tmp/gw-as-3977-22910004/tools/convert.sh wt-2
bash /tmp/gw-as-3977-22910004/tools/convert.sh wt-3

# a verifier, invoked exactly twice (boundary — NOT a finding):
bash /tmp/gw-as-3977-22910004/tools/verify.sh wt-1
bash /tmp/gw-as-3977-22910004/tools/verify.sh wt-2

# scratch paths that are DATA, not invocations — must not count:
rm -f /tmp/gw-as-3977-22910004/tools/convert.sh
mv /tmp/gw-as-3977-22910004/out/old.sh /tmp/gw-as-3977-22910004/out/old.sh.bak
> /tmp/gw-as-3977-22910004/out/report.sh

# an ephemeral per-run helper — exempt:
bash /tmp/gw-as-3977-22910004/helper.XXXXXX.sh once

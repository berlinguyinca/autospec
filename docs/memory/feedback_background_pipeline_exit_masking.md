---
name: background-pipeline-exit-masking
description: "Background Bash tasks wrapping a gate in `cmd | tail; echo` report exit 0 even when the gate failed — always parse the final status line, never trust the task-notification exit code"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 312058d2-5d79-4800-b769-be8290f0176a
---

A background task shaped `autospec validate 2>&1 | tail -5; echo "EXIT=..."` completes with exit code 0 regardless of the validator's result, because the trailing `echo` is the last command. The task-notification summary said "completed (exit code 0)" while the actual output ended in `validate: FAIL — scripts/dogfood-detectors.sh: regressions detected`. Also: `${PIPESTATUS[0]}` is bash-only; the user's shell is zsh (lowercase `pipestatus`), so the EXIT= marker printed empty.

**Why:** During the 2026-07-03 launch-readiness run, a red `validate.sh` on the rebased tree would have been pushed as "green" if only the notification exit code had been trusted.

**How to apply:** For any backgrounded gate command, read the output file and check the gate's own final status line ("OK — all validation checks passed" vs "FAIL"). Either run the gate bare (no pipeline) so the exit code is real, or grep the output for the pass marker. Related: [[per-session-worktree-isolation]].

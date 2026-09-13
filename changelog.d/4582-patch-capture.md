# 4582 — patch capture no longer depends on which state the agent left

`ProducedWork::detect` used to capture work only in the committed state: a run that
staged its changes and stopped — `staged=3 commits_ahead=0`, the common case — read as
"work produced" but `write_patch` returned `Ok(None)`, so the summary existed without
the artifact and the scratch-tree teardown deleted the only copy. Ten completed runs
were lost that way.

Capture is now state-independent:

- `detect` also captures the uncommitted states (staged, dirty, untracked) as patch
  bytes, under the same exclusions the detection ran with, without touching the index.
- `write_patch` writes the union of whichever states hold work, so every non-empty
  detection yields a non-empty patch file — a summary is never written without the
  artifact it summarises.
- `assert_captured` is the deletion guard: a scratch tree may be deleted only when its
  work is absent or durably captured; work without a non-empty patch refuses the
  deletion and names what it holds.

The executor's zero-effect capture path (`capture_work_before_zero_effect`) benefits
without change: it already refused to report work as empty, and now the work it
refuses actually survives as a patch.

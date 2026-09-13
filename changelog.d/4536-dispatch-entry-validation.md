## The dispatcher validates each queue entry before it spends a GPU slot

`autospec dispatch tick` refused a queue line that is not a positive issue
number **with its line number and content** — and a queue whose lines are
all refused is now a stall (non-zero exit), not a silent idle. A value that
names work is validated before it is spent (#4536).

- `QueueFile::parse` records refused lines as (1-based line, content) instead
  of silently dropping them: a silent skip is indistinguishable from an entry
  that was never there.
- `0` is refused: it matches `^[0-9]+$` but names no issue — the same
  discipline `autospec convert` already applies to its ISSUE argument.
- `DispatchTick` carries the refusals into its report: a per-line refusal in
  the text output, `N queue line(s) refused` in the summary, and a `refused`
  field in the `--json` output for scripted consumers.
- `DispatchTick::stalled()` is the new tick exit predicate: nothing
  dispatched over a queue that named work (skipped entries **or** refused
  lines) exits non-zero. An empty queue (no entries, no refusals) stays idle.
- The dispatch-bound tick tests moved from `dispatch_pipeline.rs` to
  `tests/queue_refusal.rs` so the ratchet-locked source file could shrink
  (2857 → 2656); new refusal tests and three CLI e2e tests cover the
  empty-string case explicitly.

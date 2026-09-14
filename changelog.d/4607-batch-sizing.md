# The conversion pass sizes its batch to its own deadline (issue #4607)

No conversion pass has ever finished its batch. Every terminal line in the
pass log across the whole history is a kill or a crash — `rc=143`,
`NO ALLOCATION`, `GATE CRASHED` — not one `rc=0`. The batch size (12) and
the deadline (7200s) were chosen independently and never compared against
the observed cost of a single patch, so each pass was killed mid-gate after
one verdict out of a dozen, and the candidates it never reached were
re-selected and re-gated from scratch on the next pass. Throughput was not
low; it was zero, and it stayed zero no matter how often the pass ran.

The fix is the invariant the issue names, not a bigger deadline (raising
the deadline only lengthens the window in which one stuck pass blocks every
scheduled successor, and still does not guarantee completion):

- **A pass sizes its work to its own deadline.** Given a deadline
  (`--deadline SECS`, or `AUTOSPEC_CONVERT_DEADLINE` in the environment —
  the scheduler that runs a scheduled pass is the one that knows the
  deadline, and the env is how it hands the pass its own time box; the
  flag, when given, is explicit and wins), the pass stops *starting* new
  patches when the remaining time is less than what a patch has been
  observed to cost in this pass, and finishes the one in flight.
- **A pass that could not reach every candidate says so.** `deferred=N` is
  part of the outcome line (`PassCounters.deferred`, accounted in
  `reconciles` like every other counter) and a `  DEFER` line announces
  it where the decisions are announced. A pass that quietly did 1 of 12
  and one that did 12 of 12 no longer print the same shape of line.
- **A deadline is not enforced by killing the work.** The pass declines to
  start what it cannot finish; the deferred patches stay on disk, keep
  their queue entries, and are re-offered next pass. Being terminated
  mid-gate is indistinguishable from a hang and discards everything the
  gate had computed.
- **Cost is measured, not assumed.** The per-item cost that governs the
  decision is the wall time of the most recently completed item in this
  pass (`convert::sizing::should_defer`), not a compiled-in constant —
  the first item on a cold cache and the tenth on a warm one differ by an
  order of magnitude, and a pass that has measured nothing starts its
  first item, whatever the deadline: that item is what produces the
  estimate.

An integration test runs the real pass against a real fixture crate and a
real cargo gate: the first patch's gate runs a test that sleeps three
seconds, the deadline is two, and the test asserts the in-flight item
finishes (`CONVERT #N`, exit 0), the second item is never started (no
verdict for it), and the outcome reads `examined=2 ... deferred=1`. A
control run without a deadline works through the whole batch and reports
`deferred=0`, and the env deadline sizes the batch the same way the flag
does.

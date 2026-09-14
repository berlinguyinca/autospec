# A health signal's first use must not be a deletion (issue #4555)

A signal is invented to answer "is this thing healthy?" — and the first thing
built on it is something that **deletes**. The signal is never checked against
an input known to be healthy and an input known to be dead. It turns out to be
wrong — sometimes exactly backwards — and the deletion is what discovers that.

## The three fleet instances, all the same shape

1. **A liveness probe that timed out on the busy workers.** The probe queued
   behind real work, so the workers that were *working* were the ones that
   "failed" the probe. The health signal marked unresponsive a worker
   generating at 48.8 tok/s with 11,080 tokens in flight.
2. **Staleness by file mtime.** Workers register once and never rewrite the
   file, so mtime measures *age*, not *liveness*. Measured live: the two
   **freshest** entries were both dead jobs; the six **stalest** were all
   RUNNING. Pruning the "stale" ones would have removed most of the fleet and
   kept both dead entries — the signal was exactly inverted.
3. **A set-difference over mis-sorted files.** `comm` on numerically-sorted
   input reported 0 candidates when the truth was nonzero; the warning went to
   stderr and the wrong answer to stdout (#4449).

In each case the signal was only ever observed on *unknown* inputs before it
gated a *destructive* action. A signal that has only ever been observed on
unknown inputs has not been observed.

## The invariants

1. **Demonstrate before gate.** A signal that gates a destructive action must
   be tested on a known-healthy input and a known-dead input — both directions,
   as tests — before it gates anything.
2. **Prefer the authoritative negative to the inferred positive.** "This job
   ID is absent from the scheduler" is a total, cheap, false-positive-free
   liveness answer; "this file's mtime is old" is a guess. Where an
   authoritative check exists, a heuristic must not override it — and usually
   should not exist.
3. **Separate detection from removal.** A detector's first release reports and
   removes nothing. Compare its verdicts against reality for a period, then
   let it act.
4. **A timestamp is liveness only if something writes it on a period.** If
   there is no heartbeat writer, mtime measures creation, and a freshness
   check over it measures age. Where freshness is required, the record carries
   an explicit `last_heartbeat` and its period, so a reader can distinguish
   "not heartbeating" from "does not heartbeat".
5. **A degraded or unmeasurable reading is `unknown`, never `bad`.** Unknown
   does not authorise removal.

## Where it is enforced

- `skills/autospec-define/prompts/decomposer-contract.md` →
  *Reaper-task decomposition*: a reaper child that does not name its source of
  truth, its healthy/dead test cases, its act-or-report first release, and its
  `unknown` semantics is filed `autospec:blocked-prerequisite`, not
  `auto-implement`.
- `skills/autospec-run/prompts/implementer-contract.md` → `REAPER_CONTRACT`
  directive: the implementer must demonstrate both directions as tests,
  separate detection from removal, and treat unknown as non-removal.

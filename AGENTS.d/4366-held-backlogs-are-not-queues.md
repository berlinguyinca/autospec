# Held backlogs are not queues (issue #4366)

Asked where the pipeline was stuck, the first answer was "121 patches
awaiting conversion, ~26 hours of serial gating — parallelise the
converter". That count was computed by listing queue entries with a
`changes.patch` on disk; a patch on disk means an agent finished, and
says nothing about whether conversion has already been attempted. The
selector that actually drives the work (`convselect`) excludes
`have_pr`, `closed_issue` and `attempted`, and offered 11 candidates.
Two sources disagreed, and the one computed by hand was the one that was
wrong — the selector's accounting line was on screen in the same session.
The real breakdown of the 116 open `auto-implement` issues that never
had a PR was 109 held (56 conflicts, 17 bats, 9 test, 6 build, 8
misc), 5 running, 2 never implemented. Nothing was waiting for converter
capacity: parallelising the converter would have sped up the 11 and done
nothing for the 109.

- **Staleness, not throughput, is what makes a fan-out pipeline stall.**
  Every agent works from a base that ages while it runs; the fix is
  shortening that window or rebasing on arrival, not adding capacity
  downstream. When the held conflicts outnumber the candidates the
  selector offers, the dominant stalled population is unreachable by any
  converter capacity (`stall_cause` → `Bottleneck::Staleness`), and a
  proposed fix must be read against how much of the stalled backlog it
  reaches (`fix_population` — "11 of 111"); a fix that does not address
  the classified stall is a finding naming both populations
  (`wrong_bottleneck_finding`).
- **Attempt a rebase before declaring a conflict.** `git apply --3way`
  against a moved base fails where `git rebase` onto the current base
  would succeed, because the latter replays intent rather than matching
  context. A conflict recorded from the apply alone is not a conflict
  yet (`conflict_without_rebase_finding`); only `Rescued` or
  `GenuineConflict` — an attempt that happened — clears it.
- **Measure the queue with the selector that drives the work.** Any
  count computed separately will disagree with what actually gets
  processed, and the separate count is the one that is wrong
  (`separate_count_finding`).
- **A held item is not a queued item.** They need opposite responses —
  one needs judgement or a fix, the other needs capacity — and
  conflating them points effort at the wrong bottleneck
  (`response_for`; a report claiming more "awaiting conversion" than the
  selector offers is a finding, `held_counted_as_queued`).

Checkable in `autospec_core::held_backlog` (`BacklogBreakdown`,
`HeldReasons`, `HoldReason`, `breakdown_line`, `reason_counts_line`,
`BacklogState`, `Response`, `response_for`, `held_counted_as_queued`,
`separate_count_finding`, `RebaseAttempt`, `conflict_without_rebase_finding`,
`Bottleneck`, `stall_cause`, `ProposedFix`, `fix_population`,
`wrong_bottleneck_finding` — pure in-memory, so the shell
`convselect.sh` / `convpass.sh` pair and the monitor log can adopt them
as the single source of truth). Tests:
`crates/autospec-core/tests/held_backlog.rs`, including the regression
that reconstructs the incident end-to-end: the 121 vs 11 miscount, the
109/5/2 breakdown, the 56 conflicts held on base staleness, and the
proposed parallelisation reaching 11 of 111.

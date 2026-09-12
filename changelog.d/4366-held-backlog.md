### Added

- `autospec-core::held_backlog` — primitives for the invariants that the
  backlog is held patches, not conversion capacity: a fan-out pipeline
  stalls on base staleness, not throughput, so a stall is classified from
  the held-conflict count against the selector's candidates
  (`stall_cause`, `Bottleneck`) and a proposed fix is read against the
  population it reaches (`fix_population` — "11 of 111"), with a fix that
  does not address the classified stall a finding naming both populations
  (`ProposedFix`, `wrong_bottleneck_finding`); a conflict is only
  declared after a rebase attempt, because `git apply --3way` against a
  moved base fails where `git rebase` succeeds (`RebaseAttempt`,
  `conflict_without_rebase_finding`); the queue is measured with the
  selector that drives the work, and a disagreeing separately computed
  count is the one that is wrong (`separate_count_finding`); and a held
  item is not a queued item — they need opposite responses, judgement or
  a fix vs capacity, and a report claiming more "awaiting conversion"
  than the selector offers is a finding (`BacklogBreakdown`,
  `HeldReasons`, `breakdown_line`, `response_for`,
  `held_counted_as_queued`) (#4366, 2026-09-11).

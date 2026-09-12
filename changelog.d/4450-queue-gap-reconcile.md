### Added

- `autospec-core::queue_gap` and `autospec dispatch queue-gap` — the
  reconciliation between the issues that should be dispatchable and the
  issues that are queued, for the invariant that a queue which stops
  accepting work must be as loud as a queue which fails to dispatch it:
  178 open issues carried the eligibility label, 90 were in the queue, 75
  already had a branch or a PR, and **37 were in none of the three sets** —
  filed, labelled, and invisible, including four filed in one session so
  agents would pick them up. Nothing rejected them, nothing held them,
  nothing compared the sets, and a queue that has quietly stopped growing
  looks exactly like a queue that is keeping up (`QueueGap::new`,
  `QueueGap::line` prints `eligible`, `queued`, `has_branch_or_pr` and
  `missing` on every run — zero included, because the zero is the evidence
  the reconciler ran); a non-empty difference is a defect that is *reported*
  and never appended away, so the reason issues stopped flowing stays
  diagnosable rather than only patched (`QueueGap::is_defect`, exit 1); and
  the counts are a partition that must add up rather than a rounded
  approximation (`QueueGap::reconciles`). The other half of the same failure
  — the loop step that told an agent to run a `refresh-queue.sh` which did
  not exist, and produced no signal at any point — is checked on the same
  run: `--require-step NAME=COMMAND` declares a component the step depends
  on and an unresolved one fails the run naming both the step and the
  command (`missing_components`, `MissingComponent::line`, on top of
  `autospec_core::procedure`), while a run that declares none says so out
  loud instead of passing quietly (`MISSING_COMPONENTS_NONE`)
  (#4450, 2026-09-12).

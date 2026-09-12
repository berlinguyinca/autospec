### Added

- `autospec-core::worktree_lock` — primitives for the invariant that a git
  checkout is not a shareable resource: any tool that mutates a checkout
  takes an exclusive lock (a file beside the worktree) and **refuses to
  run rather than queueing indefinitely**, naming the holder and the
  remedy (`lock_path`, `parse_lock_file`, `acquire`, `refusal_line` — a
  malformed lock file is fail-closed, never read as free); the lock is
  held for the whole checkout-apply-test cycle, not per command, and a
  release before the cycle completes is an error naming the phase
  (`Lease`, `Phase`, `LeaseError::PrematureRelease`); concurrent work uses
  separate checkouts, one per worker, and a shared path is a finding
  (`shared_checkout_findings`); a verdict records the checkout it was
  produced in (a verdict with no recorded checkout is refused,
  `CheckoutVerdict::new`), and a verdict whose checkout was co-held by
  another process is identifiable afterwards (`contaminated_verdicts`); a
  superseding background job stops the old one first and says so
  (`supersede`, `SupersedeOutcome`); and a pre-flight check for "is
  anything else already running against this path" refuses the second run
  outright (`preflight`) (#3608, 2026-09-11).

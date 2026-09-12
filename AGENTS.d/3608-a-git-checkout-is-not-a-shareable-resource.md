# A git checkout is not a shareable resource (issue #3608)

Two conversion runs in one worktree — each doing `git checkout -B
fix/issue-N origin/main` and `cargo test` — produced fabricated failures
for two patches that pass in isolation: patch 2605 was reported `build
error -- test failed` and patch 2607 `FAILED. 791 passed; 81 failed`, and
both passed when re-run alone (now PRs #3599 and #3600). The failure is
not a crash, it is convincing evidence in the wrong direction: a good
patch marked broken is more expensive than a broken patch marked good,
because nobody re-checks the rejected pile.

- **Any tool that mutates a git checkout takes an exclusive lock on it**
  (`flock` on a file beside the worktree, never inside it — a file inside
  would be caught by the very checkouts the lock guards) and **refuses to
  run rather than queueing indefinitely**: a conversion run is unbounded
  in length, and a queued second run that eventually starts on the same
  tree has waited for nothing. A refused invocation prints why it stopped
  and the remedy (`refusal_line`: holder, pid, since, "not queueing — run
  in a separate checkout"). A lock file that cannot be parsed is
  fail-closed, never silently read as free.
- **The lock is held for the whole checkout-apply-test cycle, not per
  command.** The lease walks `checkout → apply → test → done` in order;
  releasing before the cycle finishes is an error naming the phase it
  abandoned (`LeaseError::PrematureRelease`) — the gap between commands is
  exactly where a second process slips in.
- **Concurrent work uses separate checkouts, one per worker.** A checkout
  is cheap next to the GPU hours a wrong verdict wastes; two workers on
  the same path is a finding naming both (`shared_checkout_findings`), not
  a race to be debugged afterwards.
- **A verdict records the checkout it was produced in.** A verdict with no
  recorded checkout is refused, never defaulted (`CheckoutVerdict::new`
  returns `None`); a verdict whose checkout was co-held by another process
  is contaminated and is identifiable afterwards rather than trusted
  (`contaminated_verdicts`).
- **A superseding background job stops the old one first and says so.**
  "Start the new one and hope" is what produced the incident
  (`supersede` → `StartedNewWithoutStoppingOld`); the clean record names
  both pids and the stop order.
- **Pre-flight: is anything else already running against this path?**
  Two processes with the same working directory in the process listing is
  the whole defect; the check (`preflight`, excluding the caller itself)
  refuses the second run outright, in seconds, before it can contaminate
  anything.

Checkable in `autospec_core::worktree_lock` (`lock_path`, `LockState`,
`parse_lock_file`, `acquire`, `refusal_line`, `Lease`, `Phase`,
`LeaseError`, `shared_checkout_findings`, `CheckoutVerdict`,
`contaminated_verdicts`, `supersede`, `SupersedeOutcome`, `preflight`,
`preflight_refusal_line` — pure in-memory, no subprocess, so the shell
conversion pass owns the `flock` and supplies timestamps and process
records). Tests: `crates/autospec-core/tests/worktree_lock.rs`, including
the regression that reconstructs the incident: two processes, one
checkout — the pre-flight check finds the co-resident process, the lock
refuses the second run naming the first, and the `81 failed` verdict is
flagged contaminated by the checkout it names.

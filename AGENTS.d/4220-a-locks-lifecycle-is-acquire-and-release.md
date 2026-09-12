# A lock's lifecycle is acquire and release (issue #4220)

A per-issue claim was added to stop two conversion passes from converting
the same patch (#4214). After launching, the claim was verified — two
claims, correct owner, live process — and the lock was pronounced working.
Later, with both issues long finished (one held on conflicts, one
converted and merged), the claims were still on disk, blocking the one
issue a retry should have picked up. `_held_lock` was a single variable
overwritten each iteration, so the exit trap released only the last
claim; the rest survived until a cleanup line at the summary. **Acquisition
was verified and release was inferred.** Those are different properties,
and the one skipped is the one that makes a lock a lock rather than a
one-way marker. The consequence is narrow but real: a completed issue is
harmless — it has a PR, so the existing check catches it anyway. A *held*
issue is not: it is exactly the case a retry should pick up, and a stale
claim blocks that for the remainder of the run. The bug is invisible in
the common path and only bites the recovery path — which is the pattern
that makes it worth writing down.

- **A lock's lifecycle is acquire *and* release; testing one is testing
  half.** The assertion that matters is that the claim is gone once the
  work is done — check the claim directory after an item completes, not
  only after it starts (`lifecycle_coverage` →
  `AcquireInferredRelease` is the incident's verification itself). A
  check at run end counts for neither side: it is too late for the
  release side (the in-run retry path was already blocked) and proves
  nothing about the acquire side (a claim taken and released again is
  equally invisible at run end).
- **A resource held in a loop must be released at the top of the next
  iteration or at the end of the body, never only at function exit.** A
  single variable holding "the current lock" silently converts N locks
  into one released lock and N−1 leaks (`settle_claims`). Where the loop
  can `continue` from many places — six in this one — releasing at the
  top of the next iteration is the form with one edit site instead of six
  (`release_edit_sites`: 1 vs `continues + 1`). The leaked claims that
  actually block the recovery path are the held ones
  (`blocked_retries`): a leak on an issue that already has a PR is
  masked by the existing check, which is why the common path stayed
  green.
- **A guard added to fix a concurrency bug deserves the same scrutiny as
  the bug.** A smaller defect of the same family was introduced within
  minutes of the fix it guards. Concurrency fixes are exactly where
  "it looked right and the smoke test passed" is least sufficient: the
  guard's own claim owes the full acquire-and-release coverage above.
- **Prefer the shape that cannot leak.** A claim file whose name encodes
  the owning PID, checked for liveness on read, degrades safely on crash
  without any release path at all — the fleet's `desired.sh` uses
  one-file-per-claim for the same reason. Designing the stale state to be
  *detectable* beats remembering to clean it up (`stale_verdict`,
  `after_crash`): a PID the reader never checks is never seen
  (`PidNeverChecked`), and a claim that cannot name its owner is
  indistinguishable from a live one after a crash (`Ownerless`).

Checkable in `autospec_core::claim_lifecycle` (`lifecycle_coverage`,
`LifecycleCoverage`, `CheckPoint`, `settle_claims`, `ClaimIteration`,
`LoopClaimShape::release_edit_sites`, `ClaimLedger::{line, warn_line}`,
`blocked_retries`, `blocked_retries_line`, `stale_verdict`, `after_crash`,
`claim_file_name`, `parse_claim_file` — pure in-memory, no subprocess, so
the shell conversion pass can adopt them as the single source of truth).
Tests: `crates/autospec-core/tests/claim_lifecycle.rs`, including the
regression that reconstructs the incident: the acquire-only verification,
the six-`continue` loop released only at function exit, the on-disk
claims `3813 4131 4183` with only 4131 live, and the held issue #3813
whose retry the stale claim blocked.

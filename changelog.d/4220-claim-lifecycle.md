### Added

- `autospec-core::claim_lifecycle` — primitives for the invariant that a
  lock's lifecycle is acquire *and* release, and that testing one is
  testing half: a lock verification is classified by what it actually
  proves (`lifecycle_coverage`, `LifecycleCoverage` — a check after start
  with no check after completion is `AcquireInferredRelease`, the
  incident's own verification, and a check at run end counts for neither
  side); a claim held in a loop must be released at the top of the next
  iteration or at the end of the body, never only at function exit, and a
  single variable holding "the current lock" is settled into released and
  leaked claims per release site (`settle_claims`, `ReleaseSite`,
  `LoopClaimShape::release_edit_sites` — top-of-loop is the form with one
  edit site instead of six) with the leaked claims that actually block the
  retry path named separately (`blocked_retries`: a held issue is exactly
  the case a retry should pick up, a merged one is masked by the existing
  PR check); and the shape that cannot leak — a claim file whose name
  encodes the owning PID, checked for liveness on read, degrades safely on
  crash without any release path at all (`stale_verdict`, `after_crash`,
  `claim_file_name`, `parse_claim_file`) (#4220, 2026-09-11).

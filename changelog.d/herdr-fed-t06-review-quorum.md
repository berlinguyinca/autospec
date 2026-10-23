### Added

- `autospec review-quorum` and `autospec_core::autonomous::review_quorum`: a post-PR review quorum bound to the exact current head SHA. Verdicts for an older head are reported stale and never reused, distinct identities only, PR author / replayed receipts / non-diverse providers cannot supply an `lgtm`, and an insufficient quorum asks for re-review (Herdr federated workbench T06).

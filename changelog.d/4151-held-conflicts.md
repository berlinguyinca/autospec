### Added

- `autospec-core::held_conflicts` — primitives for the invariant that the
  fleet's own merge throughput invalidates the fleet's in-flight patches
  (#4151, 2026-09-11): 38% of held patches die on merge conflicts and
  100% of the conflicts were in files `main` changed in the same 24 h.
  A patch with no recorded base sha is refused rather than ordered
  (`InFlightPatch::new`, the converter-side half of the `rebaseline`
  sidecar); issues whose declared paths overlap are serialised, not fanned
  out, as connected components of the overlap graph with the shared
  surface named (`dispatch_batches`); patches convert in contention order
  — commits to `main` since the patch's base touching the patch's files,
  the union over files so a commit touching two counts once — not arrival
  order (`contention_order`); a pure declaration manifest — nothing but
  `mod`/`use` declarations, comments, and blanks — is the shape for which
  a union merge is safe, read from the content rather than the path
  (`declaration_manifest_findings`); and a conflict hold names the
  conflicting file's recent churn instead of inviting the reader to study
  the patch (`held_conflicts_line`), with every conflict attributed to
  the window's churn before it is read as a patch defect
  (`overlap_report`).

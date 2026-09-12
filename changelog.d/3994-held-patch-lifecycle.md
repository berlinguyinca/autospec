### Added

- `autospec_core::stored_output` dispatch-eligibility primitives: a
  produced patch is a lifecycle state, not a terminal one (issue #3994).
  The dispatcher skipped any issue whose `out/issue-N/changes.patch`
  existed, and nothing ever removed a patch — "produced" had no exit, so
  205 of 212 queued issues became undispatchable and the fleet idled at
  8/22. Encodes the fix as pure primitives the dispatch pass can adopt:
  `ApplyCheck::AppliesUnder3way` (the boundary the conversion pass
  actually uses: a patch strict `git apply --check` rejects but `git
  apply --3way` applies is still live, never superseded), `Eligibility`
  and `eligibility` (a patch that exists and applies is a skip — awaiting
  conversion; a patch that does not apply even under 3-way is retired and
  re-dispatched; a check that cannot answer is fail-closed),
  `superseded_archive_path` (retirement is archival — the patch is moved
  to `out/issue-<N>/superseded/<stem>-<timestamp>.patch`, never deleted),
  and `skip_reason_counts` / `dominant_skip_line` (a dispatch pass that
  ends with slots free and zero eligible entries logs one line naming the
  dominant skip reason and its count, so an idle fleet over a
  fully-blocked queue is visible without a manual audit).

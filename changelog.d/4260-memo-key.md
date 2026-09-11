### Added

- `autospec_core::memo_key`: memoized decisions keyed on the input, not the
  subject (issue #4260). The conversion/selector loop's "already attempted"
  filter was keyed on the issue identifier, so a redispatched agent's
  brand-new patch for the same issue was excluded forever while the
  denominator (`attempted=236 -> candidates=0`) looked healthy. Encodes the
  fix as checkable primitives — `AttemptRecord` (the attempt *and* what was
  attempted: content hash or mtime stamp; a record with no captured key is
  legacy and never excludes), `subject_keyed_excluded` /
  `input_keyed_excluded` (the failure mode and the invariant),
  `is_fresh` (recency within `DEFAULT_FRESH_MIN` overrides history, future
  mtimes fail open), `select_candidates` (disjoint-bucket `SelectionReport`
  whose `stale=N` dimension names the set the old filter would have wrongly
  excluded and `fresh=N` the recency override, with a `reconciles()`
  predicate), and `reconcile` (the end-to-end invariant: work completed
  within `DEFAULT_RECONCILE_WINDOW` must appear either as a PR or as a held
  line — per-stage counters prove the filter ran, not that the work reached
  its destination).

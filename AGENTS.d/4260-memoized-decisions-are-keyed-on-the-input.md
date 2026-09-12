# Memoized decisions are keyed on the input (issue #4260)

The conversion/selector loop kept a set of issues whose patches had "already
been attempted" and used it to exclude future candidates — keyed on the issue
identifier. When an agent was killed and redispatched, it produced a
brand-new patch for the same issue, and the filter saw "already attempted"
and excluded the fresh work forever. The denominator made the defect look
healthy: `attempted=236 -> candidates=0`. The denominator proves the filter
ran; it does not prove the filter is correct.

- **Key the exclusion on the input, not the subject.** An attempt only
  excludes the artifact it was recorded against (content hash or mtime
  stamp), never other artifacts for the same subject
  (`input_keyed_excluded`). A memo entry that never matched the current
  input must not skip it; the subject-keyed filter is kept as a named
  reference for the failure mode (`subject_keyed_excluded`).
- **Record what was attempted alongside the fact.** `AttemptRecord` carries
  the input key with the attempt. A record with no captured key is legacy
  and never excludes — re-attempting a patch is cheap, losing a fresh one
  is not.
- **Recency overrides history.** An artifact whose mtime is within the
  fresh window (`DEFAULT_FRESH_MIN`) is a candidate even when its key
  matches a recorded attempt; a future mtime (clock skew) is fresh, not an
  error. The override and the stale-memo set are first-class report
  dimensions (`fresh=N`, `stale=N`), and a `SelectionReport` whose buckets
  do not reconcile is reporting a state that cannot exist
  (`reconciles()`).
- **Per-stage counters are not end-to-end evidence.** Work completed within
  the reconciliation window (`DEFAULT_RECONCILE_WINDOW`) must appear either
  as a PR or as a held line; `reconcile` asserts that invariant and names
  — with age — every piece of work that reached neither. A PR takes
  precedence over a held line.

Checkable in `autospec_core::memo_key` (`AttemptRecord`,
`subject_keyed_excluded`, `input_keyed_excluded`, `is_fresh`,
`select_candidates`, `SelectionReport`, `reconcile`, `ReconcileReport`).
Tests: `crates/autospec-core/tests/memo_key.rs`, including the regression
case that reconstructs the incident: 62 fresh patches for 62
"already attempted" issues — the subject-keyed filter yields
`candidates=0`, the input-keyed filter yields `stale=62 ... candidates=62`.

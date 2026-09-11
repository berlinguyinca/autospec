### Added

- `autospec_core::memo_key::commit_offered`: the selector's cursor
  advancement is now a separate, explicit step (issue #4283). The
  work/conversion selector recorded what it had offered on every run, so
  calling it twice — the summary line, then the batch — silently dropped
  the candidates the first call had recorded (three completed InferWeave
  patches, #284/#288/#304, marked handled and never dispatched).
  `select_candidates` is now documented and tested as the read-only half:
  it never advances the memo, so two calls with the same input return
  identical output. `commit_offered(patches, selection, now)` returns the
  input-keyed `AttemptRecord`s for the admitted candidates, for the
  consumer that acted to append (its `--commit` flag, defaulting off).
  Regression tests in `crates/autospec-core/tests/memo_key.rs`
  reconstruct the incident and assert the twice-call equality.

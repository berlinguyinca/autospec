### Added

- `autospec-core::argument_scope` — primitives for the invariants that a
  parser must not swallow its arguments and a scoped tool must report its
  scope: a strict parser whose closed known-set rejects the first unknown
  argument as an error the caller turns into a non-zero exit (the shell `*)`
  catch-all), a rejection message that names both the offending argument and
  the correct mechanism (with a check that flags a hand-written message
  omitting either), an identical-output-across-distinct-scopes defect check
  that is the per-project loop check (identical output under the same scope
  stays an idempotent re-run, not a finding), and an environment-scope line
  that a scoped tool prints on every run, with a check that it appears in
  the output (#4292, 2026-09-11).

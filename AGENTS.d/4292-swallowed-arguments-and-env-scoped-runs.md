# Swallowed arguments and env-scoped runs (issue #4292)

`convselect.sh` was scoped to a project through environment variables
(`R=` / `OUT=` / `SEEN=`) and its argument loop had no `*)` branch:
`convselect.sh iw` was accepted, ignored, and scoped to autospec anyway.
Both invocations printed byte-identical counts — `considered=100
finished_patches=3 closed_issue=0 candidates=0 retry-held=2` — and that
identity is what hid InferWeave's real candidate (issue #288): the number
was right and the input was wrong. A parser that swallows its arguments
reports the world of whatever scope it defaulted to, and nothing in its
output says which.

- **Every argument parser has a `*)` catch-all.** Unknown arguments are
  an error the caller turns into a non-zero exit, never an ignored value.
  `StrictParser::parse` returns `Err(Rejection)` on the first unknown
  argument — the shell equivalent is the `*) echo "…" >&2; exit 2;;`
  branch.
- **The rejection names the correct mechanism, not just the error.** A
  message that says "unknown argument 'iw'" tells the caller there is a
  problem; one that says "scope with R=/OUT=/SEEN= env vars, not
  positionally" tells them the fix. `validate_rejection_message` returns
  a finding for a message that omits either the offending argument or
  the mechanism, and `Rejection::line` builds the adequate one.
- **Identical output across distinct inputs is a defect.** Two runs that
  should differ — different projects, different scopes — producing
  byte-identical output proves one input did not reach the computation.
  `identical_output_pairs` flags runs under distinct scopes with equal
  output; identical output under the *same* scope is an idempotent
  re-run, not a finding. `per_project_findings` is the combined check
  worth an explicit run anywhere a tool is invoked per-project in a loop.
- **A scoped tool prints its scope on every run.** The scope is a fact
  about the output, not an assumption the reader makes about it.
  `EnvScope::line` renders `scope: K1=V1 K2=V2 …` (key-sorted, so
  deterministic) and `reported_in` / `unreported_scope_runs` check it
  appears on a line of the run's output.

Regression tests run in the configuration the incident required: the
per-project loop with a swallowed positional argument, byte-identical
counts, and no scope line — `tests/argument_scope.rs` reproduces the
loop and asserts the three findings (one identical-output pair, two
unreported scopes), and asserts the fixed loop (env-scoped, scope
printed, `candidates=1` with `288`) produces none. Checkable in
`autospec_core::argument_scope` (`StrictParser`, `Rejection`,
`validate_rejection_message`, `EnvScope`, `identical_output_pairs`,
`per_project_findings`). Tests:
`crates/autospec-core/tests/argument_scope.rs`.

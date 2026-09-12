# A pre-filter narrower than the gate admits what the gate rejects (issue #4489)

The conversion pass pre-filters patches before the expensive gate: apply
the patch, run clippy, and only batch the survivors. The pre-filter ran
`cargo clippy -p autospec-core --all-targets` while the gate ran
`cargo clippy --workspace --all-targets`. A patch touching `autospec-cli`
therefore passed the pre-filter with `clippy=0` and carried two workspace
clippy errors into the batch — and the same patch also broke a test
(`runner_executes_the_newly_registered_bats_suites`) that the pre-filter
never ran. The batch failed, and isolating the culprit cost three bisect
runs plus a re-gate of the survivors — precisely the expense the
pre-filter exists to avoid.

A pre-filter is a prediction of the gate's verdict. Its value is entirely
in that prediction being *conservative*: it may reject something the gate
would accept (wasting one patch) but must never accept something the gate
rejects, because that is what turns a cheap check into an expensive one.
Narrowing scope for speed inverts the error direction: `-p autospec-core`
is faster than `--workspace` precisely because it examines less, and what
it does not examine is where the false accept comes from.

- **The pre-filter's scope is derived from the patch's touched crates, or
  is the full workspace — never a fixed single crate.** `crates_touched`
  reads the crate set out of the patch's paths and `derive_prefilter_scope`
  turns it into a `CheckScope`: one crate gets `-p <crate>`, several get
  all of them, and no resolvable crate falls back to `--workspace`
  (fail-closed). The incident's `-p autospec-core` on an `autospec-cli`
  patch is not a possible output of the derivation at all.
- **A batch failure reports the scope gap.** `scope_gaps` compares each
  member's recorded pre-filter scope against the gate's scope, and
  `batch_failure_line` names every member admitted at a narrower scope than
  the gate — so a later batch failure is attributed to a scope gap
  (or the gap is explicitly ruled out) instead of re-diagnosed by bisect.
- **The pre-filter and the gate share one definition of "the checks".**
  `GATE_CHECKS` is the single definition; `PREFILTER_CHECK_NAMES` is a
  subset of it, and `gate_commands` / `prefilter_commands` render both from
  the same `CheckDef` at the same `CheckScope`. The two differ only in
  which checks run (clippy before tests), never in what a check covers.

Checkable in `autospec_core::prefilter_scope` (`CheckScope`,
`crates_touched`, `derive_prefilter_scope`, `CheckDef`, `GATE_CHECKS`,
`PREFILTER_CHECK_NAMES`, `gate_commands`, `prefilter_commands`,
`BatchMember`, `ScopeGap`, `scope_gaps`, `has_scope_gap`,
`batch_failure_line` — pure in-memory, so the conversion pass can adopt it
as the single source of truth for the pre-filter's scope and the batch
failure report). Tests:
`crates/autospec-core/tests/prefilter_scope.rs`, including the regression
that reconstructs the incident end-to-end: a patch touching `autospec-cli`
derives `-p autospec-cli`, never the fixed `-p autospec-core`; a batch
whose members were admitted under narrower scopes than the `--workspace`
gate is reported with every such member named; and the pre-filter's
rendered commands are byte-identical to the gate's for the same checks.

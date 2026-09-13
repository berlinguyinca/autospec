### Fixed

- `autospec convert --apply` now verifies the tools its gate requires
  (`cargo`, `git`, `gh`) before judging any patch. A broken host — the third
  observed case of a scheduled wrapper inheriting none of a login shell's
  environment — used to report the missing tool as one `HELD` record per
  patch (28 in one run), indistinguishable in the durable ledger from
  patches that genuinely failed their gate and capable of suppressing
  re-gating of work that was never gated (#4589). The pass now exits non-zero
  with a single `FATAL: <tools> not on PATH` naming the missing tools and
  records nothing. Plan mode warns instead of refusing, and a gate wrapped
  through `AUTOSPEC_GATE_WRAPPER` requires `cargo` on the execution host
  rather than the submit host.

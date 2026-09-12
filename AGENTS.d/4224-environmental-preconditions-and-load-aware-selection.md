# Environmental preconditions and load-aware selection (issue #4224)

A fix that rests on an environmental property — a homogeneous fleet,
identical context windows, a single gateway node — is only valid while the
property holds, and nothing enforces the "while". Gateway#21 made worker
selection load-aware by picking the worker with the most context; on a
homogeneous fleet that coincided with picking the least-loaded worker, and
the fix was right. A 32k-window worker joined a fleet of 256k-window ones,
and the same code became load-blind: it stacked requests on the big-window
worker and left the free 32k worker idle. The fix had silently reverted;
nothing warned, because nothing asserted the homogeneity it depended on.

- **Assert the property in code.** A fix that rests on an environmental
  property must carry a runtime check that warns loudly when the property
  no longer holds — `window_mismatches` reports every model whose workers
  report differing context windows, and its `WARN:` line names the model
  and every window observed. An assertion that cannot run is fail-closed
  (`Observation::Unrunnable`), never read as holding. A property that lives
  only in a comment is not asserted.
- **Regression tests run in the configuration the bug required.** The bug
  required a heterogeneous fleet (mixed context windows); on a homogeneous
  fleet every selection rule agrees, so a homogeneous test cannot see the
  bug. `tests/env_preconditions.rs` instantiates the mixed-window fleet the
  incident produced.
- **Capability filter before ranking filter.** "Eligible for this request"
  (context window ≥ the request requirement, compared against the
  requirement) runs before "best among candidates" (most free slots,
  compared among workers). A ranking filter over the whole candidate set
  silently re-asserts the assumption that all candidates are equivalent —
  the property that broke. The picker is total over answering workers:
  a fleet with no eligible worker still names the least-loaded one, flagged
  not-eligible (`Selection::Selected { eligible: false }`), and holding a
  zero-free-slot worker is the separate admission decision (`admit`,
  `Verdict::HeldSaturated` vs `HeldIncapable`).
- **Record the conditions when the issue closes.** A closeout for a fix
  that rests on an environmental property carries a `Valid while:` line per
  precondition, each naming the check that re-verifies it
  (`Precondition::line`). `Precondition::new` rejects a precondition with
  no named assertion: a precondition that no check re-verifies has no
  expiry, and it is how this incident happened.

Checkable in `autospec_core::env_preconditions` (`window_mismatches`,
`select_worker`, `admit`, `verdict`, `Precondition`, `PreconditionSet`,
`evaluate`). Tests: `crates/autospec-core/tests/env_preconditions.rs`.

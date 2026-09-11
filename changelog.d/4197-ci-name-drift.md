### Added

- `autospec-core::ci_name_drift` — primitives for the invariant that a CI job's `run:`
  steps are the contract and its display name is only documentation: a job model whose
  command list is the ordered step commands (never the name), a gate-vs-steps check that
  reports a local gate out of sync with its job naming the skipped or extra step, an
  enumerating-name lint that parses the name's `(...)` group and flags it when it omits or
  adds a step name (case-insensitive), and a name re-derivation that regenerates the
  enumeration from the steps so a hand-edited name is detectable as stale — a drift in the
  gate or the name is a `WARN:` on the same pass the breakage is introduced, never a silent
  re-version (#4197, 2026-09-10).

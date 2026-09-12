### Added

- `autospec-core::prefilter_scope` — a pre-filter narrower than the gate
  admits what the gate rejects: the conversion pre-filter ran
  `cargo clippy -p autospec-core --all-targets` while the gate ran
  `cargo clippy --workspace --all-targets`, so a patch touching
  `autospec-cli` passed with `clippy=0` and carried two workspace clippy
  errors (plus a broken test the pre-filter never ran) into the batch,
  costing three bisect runs to isolate. The pre-filter's scope is now
  derived from the patch's touched crates (`crates_touched`,
  `derive_prefilter_scope` — one crate gets `-p <crate>`, several get all
  of them, and no resolvable crate falls back to `--workspace`; a fixed
  single crate cannot be produced); the pre-filter and the gate share one
  definition of "the checks" (`GATE_CHECKS`, with `PREFILTER_CHECK_NAMES`
  a subset of it and `gate_commands` / `prefilter_commands` rendering both
  from the same `CheckDef` at the same `CheckScope`, so they differ only in
  which checks run, never in what a check covers); and a batch failure
  names every member admitted at a narrower scope than the gate
  (`scope_gaps`, `batch_failure_line`), so the failure is attributed to a
  scope gap instead of re-diagnosed by bisect (#4489, 2026-09-13).

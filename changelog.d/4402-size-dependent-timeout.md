### Added

- `autospec_core::size_dependent_timeout`: a fixed timeout on a size-dependent
  operation is a silent capability filter, not a flaky check (issue #4402). A
  worker read the served context with a single `curl --max-time 10` on an
  operation whose duration scales with model size, so every model above
  roughly 30 GB (flash-next 107 GB, deepseek-v4-flash 149 GB, glm-5.3-flash
  281 GB) was excluded from the fleet while the two ~16 GB models registered —
  and because the small cases worked, nothing looked broken. The module makes
  the invariants checkable: `audit_timeout` returns `SilentCapabilityFilter`
  for a `Constant` timeout on a `ScalesWithInput` operation (a constant is
  sound only when the duration is genuinely input-independent);
  `detect_size_filter` finds the clean step in the success pattern — every
  input below a boundary succeeded, every input above it never did, both sides
  non-empty — which is the tell that a size, not a model, is doing the
  discriminating; `classify_capacity` separates `Absent` (no worker has ever
  registered) from `Failing` (one registered but is unhealthy), which the
  incident fleet could not tell apart, and gives each a distinct
  `CapacityStatus::response`; and `attribute_from_inputs` composes the size
  correlation into the attribution, so a failure that separates cleanly on an
  input property points at the harness, not the subject. Tests:
  `crates/autospec-core/tests/size_dependent_timeout.rs`, including the
  incident reconstruction (five models, the threshold at roughly 30 GB) and
  the control that an interleaved pattern is not a size filter.

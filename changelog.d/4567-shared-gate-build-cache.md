### Fixed

- `autospec convert --apply` now points every patch's gate at one shared
  build cache instead of a per-patch `target/` (#4567). The per-patch
  worktree isolates source (correct and necessary); isolating artifacts
  was pure waste, because an artifact is a pure function of its inputs
  and cargo's fingerprinting already rebuilds exactly what changed.
  Measured on the fleet: 4.4 GB of rebuilt dependency tree per patch,
  ~176 GB and 10–16 hours for a batch of 40, with the first 30 minutes
  producing no decisions and reading as a hang. The cache defaults to
  `~/.cache/autospec-convert-target`; `AUTOSPEC_CONVERT_TARGET_DIR` moves
  it, which is required when `AUTOSPEC_GATE_WRAPPER` runs the gate on
  another host. The pass announces the cache once (warm or cold) and
  every stage reports its elapsed time, because the gate is the fleet's
  rate limiter and per-patch gate cost must be visible in the log.

### Added

- A per-stage bound on the conversion gate (#4567).
  `AUTOSPEC_GATE_TIMEOUT_SECS` defaults to 1800; `0` removes the bound.
  A gate that cannot time out cannot be scheduled unattended: one
  pathological patch stalled the whole pass indefinitely, and the stall
  was indistinguishable from a hang. A stage that hits the bound is
  killed and reported with exit 124 — the code `timeout` itself uses,
  next to the 125 the wrapper reserves for "never placed". A timed-out
  stage is unmeasured, not defective: the kill is the bound firing, not
  a verdict about the change, so the patch is re-offered rather than
  held, and the recorded detail names the bound and the escape hatch.

### Documentation

- `docs/conversion-gate.md`: "The gate's build cache, and its bound".
- AGENTS.d note: a per-item sandbox must not imply a per-item build
  cache — when a spec introduces a per-item work directory for a
  buildable step, it must state which state is item-specific and which
  is shared.

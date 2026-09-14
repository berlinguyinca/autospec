# A per-item sandbox must not imply a per-item build cache (issue #4567)

The conversion pass creates a fresh worktree per patch — the right
isolation for *source* — and ran each patch's gate in that worktree with
no `CARGO_TARGET_DIR`, so cargo rebuilt the entire dependency tree for
every patch, starting from `unicode_ident`. Measured: 4.4 GB and 886
artifacts per patch, ~176 GB and an estimated 10–16 hours for a batch of
40, while the pass produced zero decisions for its first 30 minutes and
read as a hang.

The waste was not a missing optimization; it was a category error in what
"isolation" means. Artifacts are a pure function of their inputs, and
cargo already keys them that way — fingerprinting rebuilds exactly what
changed, including when a patch bumps a dependency version. Copying the
source per patch and copying the build cache per patch are different
decisions; only the first one is required.

- **Isolate what the isolation is for.** Source isolation protects the
  tree from cross-contamination; artifact "isolation" protects nothing,
  because nothing shared is at risk. When a per-item sandbox is created,
  ask what each item could contaminate — and do not sandbox what cannot
  be contaminated.
- **The per-item cost of a scheduled step is the pipeline's drain rate.**
  Conversion was the fleet's rate limiter (174 of 180 queue entries
  blocked on it), so one gate's cost multiplied across the whole backlog.
  A cost that multiplies must be measured and reported per item, or it
  will be diagnosed as a hang when it is actually just slow.
- **A scheduled step that produces no decision for 30 minutes must say
  what it is doing.** Zero output was indistinguishable from a hang; the
  diagnosis took a process-tree investigation to find a transient
  child that had already exited. A per-stage line — what is running,
  whether the build was warm, how long it took — makes the cold-build
  cost obvious immediately.
- **Bound the thing that can be scheduled unattended.** A gate that
  cannot time out cannot be left running alone: one pathological patch
  stalls the whole pass indefinitely. The bound's verdict is "unmeasured,
  not defective" — the same third outcome as a broken base — because a
  kill is the bound firing, not a claim about the change.

For specs and agent prompts: when a spec introduces a per-item work
directory for a buildable or testable step, it must state which part of
the step's state is item-specific (source, locks) and which is shared
(build cache, registries), and the implementation must keep them on
separate axes. "Each item gets its own directory" is not a complete
answer; it is the question.

Checkable in `crates/autospec-cli/src/commands/convert/gate.rs`
(`gate_target_dir`, `parse_gate_target_dir`, `cache_is_warm`,
`announce_target_dir`) and `crates/autospec-cli/src/commands/convert/gate/
gate_run.rs` (`run_bounded`, `GateTimeout`, `parse_gate_timeout`,
`TIMED_OUT_EXIT` = 124, `TIMEOUT_MARKER`, `timeout_secs_from`), with
tests: the default bound applies when unset, `0` disables it, an
unparseable value keeps the default (garbage must not silently remove
the bound), a bounded run reports finished when the command exits first,
kills and reports the reserved status when the bound fires first, an
unbounded run never times out, and the marker names the bound it fired
at. Documented in `docs/conversion-gate.md` ("The gate's build cache,
and its bound").

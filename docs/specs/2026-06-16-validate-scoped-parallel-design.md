# validate.sh — scoped + parallel execution (inner-loop speedup)

## Summary

`autospec validate` has **93 `check_*` functions** and ~30 inline `bats`
suites and runs **all of them unconditionally** — a measured **382s
(6m22s) clean wall-time, ~73% CPU (serial subprocess fan-out, not LLM)**.
Every per-issue autospec-run implementer pays the full matrix for even a
single-file change; across the ~31-PR run on 2026-06-16 that was roughly
**2.5 hours of redundant validation**.

This spec adds two **opt-in, composable** speedups that leave the default
(full, serial) behavior **byte-for-byte unchanged** so it stays the safe
merge/CI gate:

1. **`--changed [<base>]` / `--since <ref>`** — run only the checks whose
   inputs intersect `git diff --name-only <base>` (default `origin/main`),
   for the fast inner loop.
2. **`--jobs N` (parallel fan-out)** — run the independent per-skill checks
   and inline `bats` suites concurrently (bounded), composes with `--changed`.

Ground-truth correction (from the 2026-06-16 dogfooding pass): the cost is
**subprocess spawning + serial bats**, NOT LLM calls — the refine scripts are
only `bash -n`-checked. So the fix is scoping + parallelism, not "skip LLM
tests."

## Team personality

- **Selected team:** Reliability/backend — build engineer, SRE, test engineer.
- **Why this team fits:** it's a CI/gate ergonomics change where correctness of
  the *scoping logic* (never skip a check whose inputs actually changed) and
  determinism under parallelism dominate.
- **Carry into child issues:** default behavior byte-unchanged (full serial
  remains the merge gate); `--changed` must **fail safe** — when in doubt about
  whether a check is affected, RUN it; parallelism must not interleave output
  into false failures.

## Review counter-team

- **Selected counter-team:** Correctness/safety + maintainer.
- **What they should challenge:** can `--changed` skip a check that a changed
  *shared* file (e.g. `AGENTS.md`, a lib sourced by many checks) actually
  affects → a real break merges? Does `--jobs` cause flaky failures from shared
  temp files / interleaved stderr? Is the affected-set mapping maintainable as
  new checks are added (does a new check default to "always run" unless mapped)?

## Architecture

```
autospec validate
  main()  ← gains arg parsing: --changed[=<base>] | --since <ref> | --jobs N
        │
   no flags → current behavior (ALL checks, serial)  [UNCHANGED — merge gate]
        │
   --changed → compute CHANGED=$(git diff --name-only <base>...HEAD)
             → a check runs iff its declared input-globs intersect CHANGED,
               OR it is in the ALWAYS-RUN set (unmapped/shared-input checks)
        │
   --jobs N → dispatch the selected per-skill checks + inline bats via a
              bounded worker pool (background + wait, or `bats --jobs`),
              each into an isolated temp dir; aggregate exit codes
```

### Affected-set mapping (`--changed`)

- A new lib `scripts/lib/validate-affected.sh` declares, per check (or check
  group), the **input globs** it depends on. Examples: `check_lockstep`/
  `check_block_expansion` for skill `X` ← `skills/X/**`,
  `tests/fixtures/skill-goldens/X.*`; researcher checks ← `scripts/explore-research/**`.
- **Fail-safe default:** any check not in the mapping, and any check whose input
  globs include a broad/shared path that changed (`AGENTS.md`, `scripts/lib/**`,
  `autospec validate` itself, `scripts/expand-skill-blocks.sh`), is in the
  **ALWAYS-RUN** set — `--changed` NEVER silently skips a check whose
  correctness could depend on a shared input. When the changed set touches a
  shared input, `--changed` degrades to (close to) the full run and says so.
- `--changed` prints `scoped: ran <N>/<TOTAL> checks (changed: <files>)` so the
  operator sees exactly what was and wasn't run — no silent truncation.

### Parallelism (`--jobs`)

- Independent per-skill checks and inline `bats` suites run in a bounded pool
  (`--jobs N`, default = CPU-2). Each worker writes to its own temp file; the
  driver collects and prints failures deterministically (sorted), so output is
  stable regardless of completion order.
- Checks with ordering/shared-state dependencies stay serial (declared in the
  lib). Default (no `--jobs`) is fully serial = unchanged.

## Adoption (who calls the fast path)

- The merge/CI gate and the Phase 4 **final pre-merge** full-suite gate keep
  calling bare `autospec validate` (full, serial) — unchanged.
- A follow-up (out of scope here, noted) can teach the autospec-run inner loop
  to use `--changed --jobs` for the *iterative* checks while keeping the full
  run as the pre-merge gate. This spec only ships the capability + tests; it
  does not change any skill prose.

## Testing

- `tests/validate-scoped.bats` — `--changed` with a one-skill diff runs that
  skill's lockstep/golden checks and NOT unrelated skills'; a diff touching a
  shared input (`AGENTS.md`, `scripts/lib/**`) forces ALWAYS-RUN (≈full); the
  `scoped: ran N/TOTAL` line is emitted; unmapped new check defaults to run.
- `tests/validate-parallel.bats` — `--jobs N` produces the SAME pass/fail set
  and the SAME (sorted) failure output as serial on a seeded failing check; no
  temp-file collision; exit code is non-zero iff any worker failed.
- A guard test asserting **bare `validate.sh` output is byte-identical** to the
  pre-change script on a clean tree (default unchanged).

## Acceptance

- [ ] `autospec validate` accepts `--changed[=<base>]`, `--since <ref>`, and
      `--jobs N`; bare invocation is byte-for-byte unchanged (full, serial).
- [ ] `scripts/lib/validate-affected.sh` maps checks → input globs with a
      fail-safe ALWAYS-RUN set (shared inputs `AGENTS.md`, `scripts/lib/**`,
      `validate.sh`, `expand-skill-blocks.sh`, unmapped checks).
- [ ] `--changed` runs only affected + always-run checks, prints
      `scoped: ran N/TOTAL checks`, and a shared-input change degrades to the
      full set (never silently skips an affected check).
- [ ] `--jobs N` runs independent checks/bats concurrently with isolated temp
      dirs and deterministic sorted failure output; same pass/fail verdict as
      serial.
- [ ] `tests/validate-scoped.bats` + `tests/validate-parallel.bats` pass;
      `autospec validate` (full) is green and unchanged.

## Decomposition into child issues

Aiming for 2 children + umbrella.

1. **Issue A — affected-set lib + `--changed`/`--since`**:
   `scripts/lib/validate-affected.sh` + arg parsing + scoped gating in
   `validate.sh main()` + `tests/validate-scoped.bats`. Files: 3.
2. **Issue B — `--jobs` parallel fan-out**: bounded worker pool for the
   per-skill checks + inline bats, deterministic aggregation, isolated temp
   dirs + `tests/validate-parallel.bats`. Depends A (shares the new arg-parsing
   in `main()`). Files: 2.

Total: 2 children + 1 umbrella + a Phase 5.5 audit child.

## Out of scope (defer)

- Changing any skill prose to *adopt* `--changed`/`--jobs` in the autospec-run
  inner loop (separate change; this ships the capability + the full run stays
  the gate).
- A persistent check-dependency graph / caching of prior results across runs.
- Parallelizing checks with genuine shared-state ordering (kept serial).

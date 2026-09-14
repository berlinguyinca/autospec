# Pipeline coverage and the recorded gate (#4556)

The conversion pass now closes the two holes the fleet measurement exposed —
110 patches across 3 pipelines that had never been converted, not because
they were held or rejected but because the run never saw them, and a gate
that was code instead of data:

- **Coverage**: every plan and apply run reports `coverage=N/M pipelines`
  (and a `coverage` object in `--json`). A run that reached 1 of 4
  pipelines names the gaps on stderr and exits `3` — incomplete is not
  success. `--shared-llm-root` declares the shared-parent shape; the default
  treats the root as one pipeline with sibling pipelines.
- **Gate registry**: the gate a pass enforces is recorded in
  `data/convert-gate-registry.json` (or `--gate-registry PATH` /
  `$AUTOSPEC_GATE_REGISTRY`) — per-repository base branch and stage argv,
  with `@scope` expanding to the pass's affected packages. Apply mode
  refuses a repository with no recorded gate (exit `2`, before judging);
  plan mode warns. The autospec repository's registry records exactly the
  gate the pass has always run.

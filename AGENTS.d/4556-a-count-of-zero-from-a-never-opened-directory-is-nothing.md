# A count of zero from a directory that was never opened is not "nothing to convert" (issue #4556)

The conversion pass is specified over the whole pipeline glob —
`$L/*/out/issue-*/changes.patch`, every pipeline — but is run against one
pipeline's root, so it can only see that pipeline's patches. On the fleet,
110 patches across 3 pipelines had never been converted: not held, not
rejected, not recorded. The run reported `converted=0` and exit 0, and the
gaps were invisible — the pass was told one pipeline and honestly answered
for it, but the report read as the whole.

- **A run reports the scope it actually covered.** Every plan and apply run
  now carries `coverage=N/M pipelines` (and a `coverage` object in
  `--json`), and each unreached pipeline that still holds patches gets a
  `coverage gap:` line naming it and its count. An incomplete run exits `3`
  — distinct from `0` (complete), `1` (apply fatal), and `2`
  (diagnostic). Counters stay true; the exit says the run was not whole.
- **The tree shape is declared, not guessed.** A pipeline's node
  directories are structurally identical to a shared root's pipeline
  directories, so auto-detection would be a guess. `--shared-llm-root`
  declares the shared-parent shape (reached = all, complete by
  construction); without it the root is one pipeline and its siblings are
  the coverage question.
- **The gate a pass enforces is data, not code.** The gate set is recorded
  in `data/convert-gate-registry.json` in the checkout being gated (or
  `--gate-registry PATH`, else `$AUTOSPEC_GATE_REGISTRY`): per-repository
  base branch and stage argv, with `@scope` expanding to the pass's
  affected packages. Apply mode refuses a repository with no recorded gate
  — `no gate established for REPO`, exit 2, before any patch is judged —
  because a pass must not guess the gate it claims to enforce; plan mode
  warns instead, since a plan is still useful.

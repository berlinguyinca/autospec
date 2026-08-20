# End of run (fixture)

## Phase 6 — Final report

The final report writes the canonical `## Next steps` section so the operator has a
single place to look for what happens after the run.

This fixture exists because run_autospec_refine_contract reads the directive from
here rather than from the autospec trio: #3262 made /autospec a router that
delegates Phase 6 instead of documenting it.

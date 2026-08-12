# Review gap attribution

`skills/autospec-shared/scripts/emit-gaps.sh` writes structured attribution for
escaped review defects. Tests and isolated runners may set
`AUTOSPEC_REVIEW_OUTCOMES_FILE` to select the append-only outcome ledger.

Phase 5.5 passes its run boundary with `--since <ISO8601>` so only evidence from
the current run can become a self-improvement candidate. Rows without complete
PR, commit, receipt, reviewer, reasoning, diversity, and risk attribution remain
visible but cannot satisfy an experiment promotion sample.

# Autonomy v3

Autonomy v3 makes Autospec behave like a deterministic engineering organization: specialist assignment, review packets, checklist findings, review quorum, medium-risk planning, guidance requests, IDRs, learning ledger, retrospectives, memory index, policy proposals, repeated-miss issues, and council reports.

```bash
bash scripts/autospec-specialist-index.sh
bash scripts/autospec-assign-specialists.sh --dry-run --issue <number>
bash scripts/autospec-specialist-review-packets.sh --dry-run --issue <number>
bash scripts/autospec-medium-risk-plan.sh --dry-run --issue <number>
bash scripts/autospec-worker-one.sh --dry-run --issue <number> --auto-recipe
bash scripts/autospec-verify-worker-pr.sh --dry-run --pr <number>
bash scripts/autospec-run-specialist-review.sh --dry-run --pr <number>
bash scripts/autospec-review-quorum.sh --dry-run --pr <number>
bash scripts/autospec-council-report.sh --dry-run --pr <number>
bash scripts/autospec-promote-pr.sh --dry-run --pr <number>
bash scripts/autospec-retrospective.sh --dry-run
bash scripts/autospec-update-learning-ledger.sh --dry-run
```

No GitHub Actions. No scheduler. No background daemon. All autonomy is operator invoked. Medium-risk work is planned, not executed automatically.

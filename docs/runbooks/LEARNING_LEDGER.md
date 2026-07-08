# Learning Ledger

The learning ledger is repo-local memory for repeated misses, safety gaps, policy gaps, recipe gaps, adapter gaps, and verifier/worker improvements.

```bash
bash scripts/autospec-update-learning-ledger.sh --dry-run
bash scripts/autospec-policy-improvement-proposals.sh --dry-run
bash scripts/autospec-build-memory-index.sh
bash scripts/autospec-plan-repeated-miss-issues.sh --dry-run
```

Policy proposals are local files only and do not modify sibling repos.

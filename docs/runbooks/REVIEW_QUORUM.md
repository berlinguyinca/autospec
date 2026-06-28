# Review Quorum

Review quorum combines required specialist findings with verifier status before promotion to human review.

```bash
bash scripts/autospec-review-quorum.sh --dry-run --pr <number>
bash scripts/autospec-council-report.sh --dry-run --pr <number>
bash scripts/autospec-promote-pr.sh --dry-run --pr <number>
```

Quorum is an internal gate, not human approval. Blocked security/privacy/verifier findings block promotion.

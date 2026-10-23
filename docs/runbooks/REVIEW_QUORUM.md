# Review Quorum

Review quorum combines required specialist findings with verifier status before promotion to human review.

```bash
bash scripts/autospec-review-quorum.sh --dry-run --pr <number>
bash scripts/autospec-council-report.sh --dry-run --pr <number>
bash scripts/autospec-promote-pr.sh --dry-run --pr <number>
```

Quorum is an internal gate, not human approval. Blocked security/privacy/verifier findings block promotion.

## Head-bound post-PR quorum (`autospec review-quorum`)

The feature-scoped quorum above has no notion of a commit. After a PR exists, a verdict is a statement about one exact
head SHA (the executor review receipt's `review_commit`, see `executor_bridge/review_receipt.rs`), so the post-PR
quorum is evaluated by `autospec_core::autonomous::review_quorum` against the CURRENT head only:

```bash
autospec review-quorum --input quorum.json --json   # exit 0 satisfied, 1 insufficient, 3 blocked, 2 invalid
```

`quorum.json` is `{"head": "<full sha>", "policy": {"required": 2, "pr_author": "...", "excluded_identities": [],
"implementer_provider": "...", "require_provider_diversity": false}, "votes": [{"reviewer", "provider", "verdict":
"lgtm"|"blocked", "commit", "receipt_digest"}]}`. Build each vote from a receipt that `validate_review_receipt`
accepted; this command does not re-open receipts.

Rules (fail closed):

- A vote whose `commit` is not exactly the head (no prefixes) is stale: listed in `stale_reviewers`, never counted,
  and a head change after a repair therefore leaves `insufficient` with `rereview_required: true`.
- Distinct normalised identities only; the PR author and `excluded_identities` (implementer workers) cannot supply
  an `lgtm`; with diversity required a reviewer on the implementer's provider, or an unknown one, cannot either.
- An `lgtm` needs a `receipt_digest`; one digest backing `lgtm` votes of two identities voids both (replay).
- Filters only remove `lgtm` votes. A current-head `blocked` verdict always blocks, from anyone.
- A force-push back to an already reviewed SHA restores that SHA's verdicts: the SHA names the content.

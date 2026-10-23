# Review Quorum

Review quorum combines required specialist findings with verifier status before promotion to human review.

```bash
bash scripts/autospec-review-quorum.sh --dry-run --pr <number>
bash scripts/autospec-council-report.sh --dry-run --pr <number>
bash scripts/autospec-promote-pr.sh --dry-run --pr <number>
```

Quorum is an internal gate, not human approval. Blocked security/privacy/verifier findings block promotion.

## Head-bound post-PR quorum (`autospec review-quorum`)

The feature-scoped quorum above has no notion of a commit. After a PR exists a verdict is a statement about one exact head
SHA of one review target, so the post-PR quorum is evaluated by `autospec_core::autonomous::review_quorum` against the
CURRENT head only. There is ONE policy, implemented twice (this Rust evaluator for merge gating, Herdr's
`/api/v1/graph/nodes/{pr}/review-quorum` for the workbench) and pinned by a shared vector file,
`crates/autospec-core/tests/fixtures/review-quorum-vectors.json`, loaded by this crate's tests and by a byte-identical copy
in herdr-engineering (`tests/control_plane/fixtures/`). Change a rule in the vectors first.

```bash
autospec review-quorum --input quorum.json [--json] [--write-report .autospec/reports/review-quorum.json]
# exit 0 satisfied, 1 insufficient (re-review needed), 3 blocked, 2 invalid input
```

`quorum.json`: `{"target": "pr:github.com/o/r#7", "head": "<full sha>", "policy": {"required", "pr_author" (required),
"excluded_identities", "eligible_reviewers", "reviewer_groups", "reviewer_logins", "implementer_provider",
"require_provider_diversity"}, "votes": [{"reviewer", "target", "verdict", "commit", "receipt_digest",
"receipt_binding", "provider"}]}`. Build each vote from a receipt that `validate_review_receipt` accepted.

Rules (fail closed):

- Blocks are processed first and never dropped. A current-head `blocked` blocks whoever filed it, unless `eligible_reviewers`
  is non-empty and the voter is not on it (Herdr always has such a list). A block with a malformed reviewer id, an unparseable
  commit or a prefix of the head fails closed (blocks); a block for another head is stale; for another target, ignored.
  Targets compare case-insensitively.
- An `lgtm` counts only for this target and EXACTLY this head, from a valid eligible identity that is not the author
  (`x[bot]` equals `x`; `reviewer_logins` maps a key to a login) nor an excluded worker, with a 64-hex `receipt_digest` and a
  `receipt_binding` that recomputes: `sha256("pr-review-receipt-v1\0" lower(target) "\0" commit "\0" reviewer "\0" digest)`.
- Verdicts, identities and digests are trimmed and lower-cased. `reviewer_groups` entries count once. One digest backing
  `lgtm` votes of two groups voids both (replay). A missing `pr_author` is an error, never a pass.
- A force-push back to an already reviewed SHA restores that SHA's verdicts: the SHA names the content.

Residual (honest): the binding shows a stored vote was not copied across target, commit or reviewer; it is not a signature,
so a colluding key can recompute it. Two keys of one person count twice unless listed in a `reviewer_groups` entry. Provider
diversity is a normalised string compare here (the harness-aware `providers_are_diverse` lives in the executor bridge).
A verdict is bound to the head, not the base.

### Closing the loop with Herdr (off by default)

Config (YAML only, chmod 600 because it holds the reviewer's own ingest key; the key travels to `curl` in a 0600 header file,
never argv):

```yaml
herdr:
  enabled: false          # true to publish / request
  url: https://herdr.example.test
  ingest_key: <this reviewer's own ingest key>
  curl: curl
  dispatch_command: /path/to/your/review-dispatch   # called as: <cmd> <pr-node> <head>
```

- Producer: `autospec review-quorum publish --config C --request R --verdict V --pr-node pr:github.com/o/r#7` after
  `independent-review-adapter.sh validate` printed LGTM (a `blocked` verdict is always publishable). It POSTs a `review:*`
  node and a `reviewed` edge bound to the verdict's commit, with the receipt digest and its binding. Deterministic event ids
  make a re-publish a Herdr `duplicate`.
- Consumer: `autospec review-quorum request --config C --pr-node pr:github.com/o/r#7 [--state-dir D]` (run it from the
  same cron/loop that polls the PR) asks Herdr for the quorum and, when `rereview_required` is true for the current head, runs
  `dispatch_command` exactly once per (PR, head) (marker file per pair; a failed dispatch is retried). Point `dispatch_command`
  at the existing review dispatch (e.g. a wrapper over `independent-review-adapter.sh prepare`).
- Merge path: `--write-report` writes the `review-quorum.json` that `scripts/autospec-promote-pr.sh` already gates on, so
  promotion is blocked unless THIS evaluator says `pass`; no shell changed (the per-file shell ratchet only falls). Run it
  after the feature-scoped Python quorum, which writes the same file.

The Python feature-scoped `review-quorum` is unchanged and is a porting candidate (#4447).

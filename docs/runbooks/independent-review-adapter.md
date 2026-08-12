# Independent review adapter

`skills/autospec-run/scripts/independent-review-adapter.sh` prepares and validates
the evidence contract used before an autonomous merge.

- `prepare` writes a request bound to the repository, issue, PR, and exact commit.
- `validate` accepts only one structured verdict matching that request.
- Integration-risk work must cite at least one integration path.
- Critical-risk work requires a reviewer from a distinct known provider.
- Missing foreground review capability requeues the issue and never merges it.

The adapter is invoked by `skills/autospec-run/scripts/invoke-review.sh`; operators
normally do not call it directly.

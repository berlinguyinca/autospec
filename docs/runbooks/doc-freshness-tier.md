# Documentation freshness tier

The documentation freshness tier is the CI-facing wrapper for issue #1540. It
keeps docs, examples, and agent-ingest exports in one verified path instead of
letting each check drift independently.

## Gate command

Run the gate against a pull request:

```bash
bash skills/autospec-shared/scripts/doc-freshness-tier.sh --pr 123 --repo-root "$PWD"
```

For local or test runs, use `--working-tree` or `--diff <file>`. Add `--dry-run`
to print the doc-update issue that would be filed without calling GitHub.

## What it enforces

1. `check-doc-drift.sh` still detects public API/config/flag drift and missing
   documentation scopes.
2. `example_stale` findings and failing changed-doc examples block the merge.
3. Changed docs run `verify-examples.mjs`, then regenerate `llms.txt` and
   `llms-full.txt` through `gen-llms-txt.sh --repo-root <repo>`.
4. Public-surface drift proposes or files a follow-up issue labeled
   `auto-implement` so doc updates re-enter the normal autospec-run queue.

If `llms.txt` or `llms-full.txt` changes during the gate, commit those exports
with the documentation change and re-run the gate.

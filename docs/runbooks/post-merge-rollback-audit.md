# Post-merge rollback audit runbook

Autonomous merge-to-main recovery is driven by `scripts/autonomous-resilience.sh post-merge-health`.
After a merge, pass the merge provenance JSON that was produced by the premerge gate:

```bash
scripts/autonomous-resilience.sh post-merge-health \
  --repo OWNER/REPO \
  --provenance .autospec/autonomous/OWNER__REPO/provenance/pr-123.json
```

The command polls the main-branch health signal. A green signal appends a
`post_merge_healthy` event to the merge audit log. A red signal dispatches the
rollback handle from provenance, files a follow-up issue with the health evidence,
and appends a `post_merge_rollback` event.

The audit log defaults to `~/.autospec/autonomous/<owner>__<repo>/merge-audit.jsonl`.
For deterministic tests or dry-run harnesses, override it with:

```bash
AUTOSPEC_MERGE_AUDIT_LOG=/tmp/merge-audit.jsonl \
  scripts/autonomous-resilience.sh post-merge-health --repo OWNER/REPO --provenance provenance.json
```

Query the log without parsing state directories by hand:

```bash
scripts/autonomous-resilience.sh audit-log query --repo OWNER/REPO --pr 123
scripts/autonomous-resilience.sh audit-log query --repo OWNER/REPO --event post_merge_rollback
```

Rollback and issue filing are seamable for tests:

- `AUTOSPEC_ROLLBACK_CMD` receives `--repo`, `--handle`, and `--pr`.
- `AUTOSPEC_GH_CMD` replaces `gh` for health polling and follow-up issue creation.

Rollback does not prompt. If the follow-up issue command fails, the audit event is
still written with `followup_issue_url: null` so operators can query incomplete
recovery evidence.

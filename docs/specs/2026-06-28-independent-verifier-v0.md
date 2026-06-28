# Independent Verifier v0

The verifier is the no-self-approval layer for Autospec worker output. It is independent from the worker path: it reads worker artifacts, issue context, PR metadata when available, and generated reports, then produces a deterministic verdict for human review.

## Command

```bash
bash scripts/autospec-verify-worker-pr.sh --dry-run --issue <number>
bash scripts/autospec-verify-worker-pr.sh --dry-run --pr <number>
bash scripts/autospec-verify-worker-pr.sh --confirm --pr <number>
bash scripts/autospec-verify-worker-pr.sh --dry-run --work-item .autospec/state/work-items/<id>
```

Dry-run is the default. Confirm mode may post a PR comment. The verifier never approves, merges, pushes, or edits worker output.

## Inputs

The verifier reads available worker and Autospec reports:

- `.autospec/state/published-issues.json`
- `.autospec/state/implementation-packet.md`
- `.autospec/reports/worker-risk-classification.json`
- `.autospec/reports/worker-validation-plan.json`
- `.autospec/reports/worker-validation.json`
- `.autospec/reports/worker-diff-review.json`
- `.autospec/reports/worker-result.json`
- `.autospec/reports/baseline-composition.json`
- `.autospec/reports/baseline-gap-analysis.json`
- `.autospec/reports/constitutional-gap-report.json`

When invoked with `--pr`, GitHub CLI calls are isolated behind the command boundary and can be stubbed in tests.

## Dimensions

Verifier v0 evaluates issue alignment, acceptance criteria, constitution traceability, baseline traceability, risk classification, patch budget, forbidden paths, test evidence, validation evidence, documentation sync, metadata sync, PR body completeness, human readability, and stuck/follow-up handling.

Each dimension reports `pass`, `warn`, `fail`, or `unknown` with evidence and required action.

## Verdicts

Verifier v0 produces one of:

- `pass`
- `pass_with_warnings`
- `needs_changes`
- `blocked`
- `needs_guidance`

Forbidden path or severe risk failures block. Missing validation for code changes needs changes. Missing issue alignment needs guidance.

## Outputs

- `.autospec/reports/verifier-report.json`
- `.autospec/reports/verifier-report.md`
- `.autospec/state/verifications/<issue-or-pr-id>.json`
- `.autospec/state/verifications/<issue-or-pr-id>.md`

Markdown is the primary human output and includes a verdict, summary, source issue/PR, verification matrix, acceptance criteria review, risk and budget review, validation evidence, docs/metadata sync, findings, required actions, and recommended next step.

## Safe Defaults

```yaml
autonomy:
  verifier:
    enabled: true
    default_mode: dry_run
    require_confirm: true
    comment_on_pr: false
    apply_labels: true
```

No GitHub write is allowed without `--confirm`. v0 uses PR comments only, not formal review approval or request-changes APIs.

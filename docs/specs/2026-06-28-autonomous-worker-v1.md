# Autonomous Worker v1

Autospec worker v1 is the first code-change gate. It does not make broad autonomous edits. It classifies one planned issue, checks whether low-risk code work is explicitly enabled, writes an implementation packet, selects validation, inspects the local diff, and produces PR evidence or a stuck handoff.

## Enablement

Code changes are disabled by default. Enable only bounded low-risk mode:

```yaml
autonomy:
  worker:
    allow_code_changes: true
    code_change_mode: low_risk_only
    max_files_changed: 8
    max_code_files_changed: 4
    max_lines_changed: 300
    max_test_files_changed: 4
    max_new_dependencies: 0
    forbidden_paths:
      - .env
      - .env.*
      - "**/*secret*"
      - "**/*credential*"
      - "**/migrations/**"
      - ".github/workflows/**"
    require_tests_for_code: true
    require_validation: true
```

Safe defaults apply when config is absent: dry-run behavior, code changes disabled, zero new dependencies, secret paths forbidden, migrations forbidden, and workflow/deployment changes forbidden.

## Risk Classification

Worker v1 writes:

- `.autospec/reports/worker-risk-classification.json`
- `.autospec/reports/worker-risk-classification.md`

Supported classifications are `docs-only`, `spec-only`, `metadata-only`, `test-only`, `low-risk-code`, `medium-risk-code`, `high-risk-code`, `architecture-required`, `blocked`, `needs-guidance`, and `unsupported`.

Only `low-risk-code` may proceed through code-change gates, and only when `allow_code_changes: true`.

## Eligible Work

Eligible examples:

- small script bug fix
- validation helper improvement
- report formatting improvement
- test fixture update
- small CLI option
- non-breaking parser enhancement
- deterministic metadata/report generation fix

Ineligible examples:

- auth, permissions, or security model changes
- database migrations
- deployment or workflow rewrites
- dependency upgrades
- new AI provider integrations
- large refactors
- public API breaking changes

## Reports

Worker v1 writes:

- `.autospec/state/implementation-packet.md`
- `.autospec/reports/worker-validation-plan.json`
- `.autospec/reports/worker-validation-plan.md`
- `.autospec/reports/worker-diff-review.json`
- `.autospec/reports/worker-diff-review.md`
- `.autospec/reports/worker-pr-body.md`
- `.autospec/reports/worker-stuck-handoff.md` when refused or blocked

The implementation packet includes risk classification, code-change eligibility, patch budget, test-first plan, expected files, forbidden files, validation plan, rollback plan, and stuck criteria.

The diff review checks changed files, added/removed lines, forbidden paths, patch budget, test/docs metadata coverage, planned-vs-actual file drift, and whether PR creation is allowed.

## Stuck Fallback

Worker v1 refuses unsafe work and writes a stuck handoff explaining why it refused, required capability level, suggested split, safer first issue, and human decision needed.

Run:

```bash
bash scripts/autospec-worker-v1.sh --dry-run
```

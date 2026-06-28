# Autospec Constitution MVP

## What is included

- Structured policy loading, Digital Twin, rule checks, doctrine audits, spec coverage, issue-plan-v3, onboarding, bootstrap, and local autonomy controls.

## What is intentionally not included

- GitHub Actions, schedulers, auto-merge, self-approval, automatic dependency upgrades, migrations, and auth/security behavior changes.

## Companion repositories

- `autospec-constitution`
- `autospec-baselines`

## Operator-invoked safety model

Dry-run is default. Confirmed writes require `--confirm`.

## Core commands

- `bash scripts/autospec-release-candidate-gate.sh --dry-run`
- `bash scripts/autospec-dogfood-rc.sh --dry-run`
- `bash scripts/autospec-mvp-status.sh`

## Existing repo onboarding

Use `scripts/autospec-onboard-existing-repo.sh`.

## New project bootstrap

Use `scripts/autospec-bootstrap-new-project.sh`.

## Constitution audit

Use `scripts/autospec-constitution-audit.sh`.

## Digital Twin

Use `scripts/autospec-build-digital-twin.sh`.

## Issue planning and publishing

Use `scripts/autospec-audit-to-backlog.sh --dry-run` before any confirmed publishing.

## Worker/verifier/supervisor

Use supervisor dry-run before confirmed worker execution.

## AI/NLAI/product scaffolds

Scaffolds generate target-repo specs and issue drafts, not arbitrary runtime implementations.

## Doctrine audits

Run `scripts/autospec-doctrine-audit.sh --dry-run --all`.

## Known limitations

See `docs/KNOWN_LIMITATIONS.md`.

## Upgrade/migration notes

Structured reports remain backward compatible with older heuristic reports where possible.

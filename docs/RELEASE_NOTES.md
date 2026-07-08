# Autospec Local Validation Foundation

## What is included

- Structured policy loading, Digital Twin, rule checks, doctrine audits, spec coverage, issue-plan-v3, onboarding, bootstrap, local autonomy controls, runtime evidence automation, and Autonomy v3 specialist/quorum/learning governance.
- Release-candidate closure reclassified the last non-green engine rows with concrete evidence: Digital Twin surfaces and knowledge graph are implemented by the Digital Twin builder, documentation drift detection is implemented by metadata drift validation, and no-self-approval is enforced by verifier/promotion/supervisor side-effect gates.

## What is intentionally not included

- GitHub Actions, schedulers, auto-merge, self-approval, automatic dependency upgrades, migrations, auth/security behavior changes, automatic medium-risk execution, sibling-repo policy modification, real multi-agent LLM execution, quorum bypass, verifier bypass, and automatic resume without guidance.

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

The Digital Twin builder generates API/UI/data/settings/permission/AI/MCP surface maps, knowledge graph state, and the Digital Twin summary. Fresh clones should run the builder before treating generated `.autospec/state/*` metadata as current.

## Issue planning and publishing

Use `scripts/autospec-audit-to-backlog.sh --dry-run` before any confirmed publishing.

## Worker/verifier/supervisor

Use supervisor dry-run before confirmed worker execution.

Autonomy v3 adds deterministic specialist assignments, review packets, checklist findings, review quorum, medium-risk plans, guidance requests, IDRs, learning ledger, policy proposals, retrospectives, memory index, repeated-miss planning, council reports, and v3 status dashboards. These are review/planning surfaces, not merge or approval authority.

## AI/NLAI/product scaffolds

Scaffolds generate target-repo specs and issue drafts, not arbitrary runtime implementations.

## Doctrine audits

Run `scripts/autospec-doctrine-audit.sh --dry-run --all`.

## Known limitations

See `docs/KNOWN_LIMITATIONS.md`.

Implemented means engine support exists and is covered by local commands/reports. Scaffolded means target-repo runtime behavior still needs target-specific implementation. Deferred means intentionally outside local validation foundation or requires human-guided future work.

## Upgrade/migration notes

Structured reports remain backward compatible with older heuristic reports where possible.

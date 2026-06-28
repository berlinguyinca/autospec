# Known Limitations

## Implemented in engine

- Structured policy source loading, validation, lockfiles, baseline composition, rule extraction, rule checks, quality gates, maturity scoring, and issue-plan-v3 generation.
- Digital Twin v1 metadata generation, knowledge graph foundations, impact analysis, and metadata drift reports.
- Existing repository onboarding and new project metadata-first bootstrap.
- Local supervisor cycle/loop, worker v0/v1/v2 rule-progress evidence, verifier, promotion gate, stuck/guidance flow, budgets, locks, stop/resume, and status reports.
- Release-candidate diagnostics: preflight, MVP smoke, command audit, report index, state validation, sensitive-output audit, recovery status, and generated-report cleanup.

## Implemented as target-repo scaffolds

- AI platform, provider abstraction, OpenAI-compatible/Ollama support, AI settings, RAG assistant, token/cost tracking dashboard, MCP diagnostics, NLAI capability interface, and pretty rendering.
- Product baseline features: in-app documentation center, settings area, onboarding tutorials, reporting dashboard, analytics metrics, feedback/support flow, diagnostics/status page, and visual design system.
- Tutorial/PDF/reporting/visualization expectations are scaffolded as specs and issue drafts for target repositories, not implemented as generic app runtime code.

## Validated by policy/rules only

- Design-pattern discipline, ADR expectations, dependency governance, modernization planning, migration discipline, Playwright viewport/screenshot expectations, visual/accessibility/performance/migration testing doctrine, reporting chart-selection standards, and documentation drift expectations.
- These are represented by structured rules/checks/gates and may generate backlog issues, but many require target-repository implementation.

## Deferred beyond MVP

- Full AI assistant implementation in arbitrary target repositories.
- Full NLAI runtime implementation in arbitrary target repositories.
- Automatic dependency major upgrades.
- Automatic database migrations.
- Automatic auth/security behavior changes.
- Full visual UI generation.
- Scheduled/background automation.
- Auto-merge and self-approval.

## Not supported by design

- GitHub Actions installation by Autospec.
- Cron or background scheduler setup.
- Direct default-branch pushes.
- Production secret storage in reports, metadata, templates, or generated docs.
- Unbounded autonomous loops.

## Requires human guidance

- Auth, authorization, permissions, privacy/security policy, billing/payments, migrations, data deletion, deployment/infrastructure, major dependency upgrades, framework migrations, public API breaking changes, and multi-service behavior.
- Worker v1/v2 remains bounded to docs/spec/metadata/test and low-risk code work unless a human explicitly narrows and approves the risk.

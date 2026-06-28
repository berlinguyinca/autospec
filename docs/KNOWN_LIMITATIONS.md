# Known Limitations

## Implemented in engine

- Structured policy source loading, validation, lockfiles, baseline composition, rule extraction, rule checks, quality gates, maturity scoring, and issue-plan-v3 generation.
- Digital Twin v1 metadata generation, knowledge graph foundations, impact analysis, and metadata drift reports.
- Existing repository onboarding and new project metadata-first bootstrap.
- Local supervisor cycle/loop, worker v0/v1/v2 rule-progress evidence, verifier, promotion gate, stuck/guidance flow, budgets, locks, stop/resume, and status reports.
- Release-candidate diagnostics: preflight, MVP smoke, command audit, report index, state validation, sensitive-output audit, recovery status, and generated-report cleanup.
- Spec coverage implementation sweep, architecture governance, UI/UX, Playwright evidence, documentation artifact, reporting/analytics, AI platform, NLAI, diagnostics, dependency governance, modernization, security/privacy, and unified doctrine audits.
- Autonomy v2 worker capability registry, implementation recipe registry, rule-to-recipe planner, patch-plan builder, template application, stack-profile detection, recipe-backed worker dry-run execution, rule recheck, and recipe-aware verifier/supervisor reporting.
- Target-App Runtime Implementers v1 for recognized stacks: runtime adapters, feature slices, runtime plans, bounded runtime shell generation, Playwright evidence generation, metadata sync, runtime verification, worker v4 dispatch, verifier v5 runtime review, supervisor runtime visibility, and runtime status.

## Implemented as target-repo scaffolds

- AI platform, provider abstraction, OpenAI-compatible/Ollama support, AI settings, RAG assistant, token/cost tracking dashboard, MCP diagnostics, NLAI capability interface, and pretty rendering.
- Product baseline features: in-app documentation center, settings area, onboarding tutorials, reporting dashboard, analytics metrics, feedback/support flow, diagnostics/status page, and visual design system.
- Tutorial/PDF/reporting/visualization expectations are scaffolded as specs and issue drafts for target repositories, not implemented as generic app runtime code.
- Architecture, UI/UX, testing, documentation, reporting, AI/NLAI, diagnostics, dependency, and security/privacy templates provide target-repo implementation plans. They are not generic runtime implementations.
- Product/AI/NLAI feature recipes can generate specs, metadata plans, issue drafts, templates, and bounded scaffolds. Stack-specific runtime files are generated only when the target stack is confidently detected and the recipe/capability permits it.
- Recognized-stack runtime shells are implemented for product, AI, NLAI, reporting, and Playwright evidence slices. They are shell/partial implementations unless tests and metadata prove complete behavior.

## Validated by policy/rules only

- Design-pattern discipline, ADR expectations, dependency governance, modernization planning, migration discipline, Playwright viewport/screenshot expectations, visual/accessibility/performance/migration testing doctrine, reporting chart-selection standards, and documentation drift expectations.
- These are represented by structured rules/checks/gates and may generate backlog issues, but many require target-repository implementation.
- Doctrine audits add heuristic evidence collection and local issue drafts. A pass means Autospec found evidence; it is not a substitute for human review on high-risk product/runtime behavior.

## Deferred beyond MVP

- Full AI assistant implementation in arbitrary target repositories.
- Full NLAI runtime implementation in arbitrary target repositories.
- Automatic dependency major upgrades.
- Automatic database migrations.
- Automatic auth/security behavior changes.
- Full visual UI generation.
- Arbitrary target-app runtime feature generation without a recognized stack, bounded recipe, patch plan, validation path, and verifier review.
- Full end-to-end target-app runtime implementation beyond shell/partial slices.
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
- Worker v3 recipe execution remains bounded by the worker capability registry, stack profiles, patch budgets, and verifier review.
- Worker v4 runtime feature execution remains opt-in through explicit feature invocation; supervisor runtime generation is disabled by default.

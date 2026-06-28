# Autospec Constitution Master Spec

## Vision

Autospec turns policy into action: the Constitution defines doctrine, Baselines define playbooks, and the engine builds repository intelligence, produces structured backlog, and runs bounded autonomous implementation only where safety gates allow it.

## Repository split

- `autospec-constitution`: source of doctrine and structured rules.
- `autospec-baselines`: source of baseline packs, profiles, capabilities, quality gates, and issue templates.
- `autospec`: engine, local operator commands, metadata generation, issue planning, worker/verifier/supervisor, and reports.

## Constitution

The Constitution stores human-readable doctrine plus machine-readable structured rules. Rules define severity, maturity, applicability, check type, evidence requirements, remediation hints, acceptance criteria, and risk metadata.

## Baselines

Baselines compose application, technology, AI, governance, testing, reporting, and operations packs into effective capability and rule expectations for a repository profile.

## Engine

The engine loads local policy sources, validates them, locks reproducible inputs, composes baseline packs, extracts structured rules, resolves effective rules with waivers/opt-outs, checks rules against repository metadata, generates gap reports, scores maturity, and drafts issue-plan-v3 backlog.

The engine also runs doctrine audits for architecture governance, UI/UX, Playwright evidence, documentation artifacts, reporting/analytics, AI platform, NLAI, diagnostics, dependency governance, modernization planning, and security/privacy. These audits are local, heuristic, evidence-based, and produce reports plus local backlog drafts without GitHub writes.

Runtime Evidence & Product Quality Automation adds local proof artifacts for generated target-app slices: app launch profiles, confirm-only app harness runs, Playwright evidence, responsive screenshots/contact sheets, visual polish audits, accessibility evidence, tutorial artifacts, PDF/report artifact plans, mock-only AI/NLAI simulations, token usage evidence, evidence bundles, and product quality scorecards. These are operator-invoked and never install dependencies or call external AI providers by default.

## Digital Twin

The Digital Twin summarizes repository inventory, technologies, capabilities, surfaces, settings, permissions, AI/MCP indicators, domain model, workflow map, knowledge graph, impact analysis, and metadata drift.

## Metadata

Autospec stores version-controlled repository intelligence under `.autospec/state` and human-readable reports under `.autospec/reports`. Generated metadata must be deterministic, evidence-scored, and free of raw secrets.

## Autonomous control plane

Autonomy is local/operator-invoked. Dry-run is default. Confirm is required for writes. The control plane includes issue publishing/sync, labels/state machine, locks, budgets, stop/resume, stuck/guidance, remediation planning, supervisor cycle, supervisor loop, status, preflight, smoke, and recovery reports.

## Worker/verifier/supervisor

The worker processes one issue at a time, only within strict risk and patch budgets. The verifier independently checks worker evidence, policy traceability, rule progress, quality gates, validation, docs/metadata sync, and PR body completeness. The supervisor selects one eligible issue per cycle and never merges or approves.

## Existing repo onboarding

Existing repositories get read-first onboarding: Digital Twin, technology/capability metadata, confidence summaries, clarification drafts, constitutional gaps, maturity score, and safe next commands before implementation.

## New project bootstrap

New repositories get metadata-first bootstrap: Autospec config, product purpose, personas, roadmap, domain/workflow maps, architecture map, registries, quality dashboard, project blueprint, and initial implementation plan.

## Product baseline features

Target repositories should eventually include in-app documentation, settings, onboarding/tutorials, reporting, analytics, feedback/support, diagnostics/status, search/help, and admin/operations areas where appropriate. Autospec scaffolds these as specs and issue drafts, and Target-App Runtime Implementers v1 can generate bounded shell/partial runtime slices for recognized React/Vite and Next.js stacks.

## AI platform

AI platform expectations include provider abstraction, OpenAI-compatible APIs, Ollama/local support, model selection, AI settings/admin pages, RAG, embeddings, agent/tool registries, memory, MCP registry, token/cost tracking, usage dashboards, quotas/budgets, and audit logging. Autospec scaffolds these for target repositories, validates them through rules, and can generate safe AI settings/RAG/token-dashboard/MCP shell slices without provider calls, secret persistence, or migrations.

## NLAI

Natural-language application interfaces expose core app capabilities through safe tools, support data querying, SQL generation/explanation/visualization, file discovery/preview/operations, workflow execution, report generation, pretty rendering, citations, evidence, and raw JSON avoidance. Autospec scaffolds and validates these expectations, and can generate NLAI shell/viewer slices without executing SQL or destructive file operations.

## Diagnostics

Diagnostics expectations include health, logs, metrics/traces, frontend white-screen diagnosis, Playwright repro, console/network capture, MCP diagnostics, incident reports, and safe remediation boundaries.

## Testing/tutorial/PDF/reporting

Testing doctrine includes unit, integration, contract, e2e, visual, accessibility, performance, migration, deterministic tests, focused validation, and evidence capture. Tutorials, screenshots, PDF guides, report formatting, and documentation drift detection are represented as rules and target-repo scaffolds.

Runtime evidence reports must separate proof from plans: Playwright evidence requires existing Playwright tooling, PDF generation requires existing PDF tooling, visual polish is heuristic, accessibility evidence is not certification, and AI/NLAI simulations are mock-only unless an operator explicitly configures safe provider calls outside the default model.

## UI/UX

UI/UX doctrine includes mobile/tablet/desktop responsiveness, accessibility, keyboard/touch behavior, design tokens, visual hierarchy, empty/loading/error states, pretty output, and raw JSON avoidance.

## Security/privacy/operations

High-risk auth, authorization, permissions, secrets, encryption, billing, payments, migrations, data deletion, deployment, infrastructure, public API breaking changes, privacy/security policies, and multi-service behavior require human guidance.

## Continuous evolution

Autospec continuously compares policy expectations to repository reality through rule audits, spec coverage, backlog generation, worker/verifier evidence, smoke reports, and release readiness reports.

Spec coverage implementation sweeps read the master requirements inventory and classify remaining work into safe engine work, rule checks, scaffolds, templates, fixtures, target-repo work, human-guidance work, or beyond-MVP deferral. They do not implement arbitrary target application features inside Autospec.

## Implemented MVP scope

The MVP implements structured policy loading/checking, Digital Twin v1, issue-plan-v3, issue publishing/sync, worker/verifier/supervisor, onboarding, bootstrap, scaffolds, doctrine audits, local control, status, smoke, recovery, sensitive-output audit, spec coverage closure, Autonomy v2 recipes, and bounded target-app runtime shell generation for recognized stacks.

## Beyond-MVP scope

Beyond MVP includes full target-app AI/NLAI runtime generation, automatic major upgrades, migrations, auth/security changes, deployment automation, full visual UI generation, scheduled/background autonomy, and auto-merge/self-approval. These remain deferred or unsupported by design unless future policy explicitly changes.

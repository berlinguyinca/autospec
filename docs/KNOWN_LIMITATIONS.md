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
- Runtime Evidence & Product Quality Automation v1: app launch profile detection, confirm-only app harness planning, operator-invoked Playwright evidence runs, screenshot contact sheets, visual/accessibility evidence audits, tutorial artifacts, PDF/report artifact planning, mock-only AI/NLAI simulation, token usage evidence, evidence bundles, product quality scorecards, verifier v6 evidence review, worker v5 evidence planning, and supervisor v5 evidence readiness.
- Autonomy v3 specialist governance: deterministic specialist role registry, assignments, review packets, checklist findings, review quorum, medium-risk planning, guidance requests, implementation decision records, learning ledger, policy improvement proposals, retrospectives, memory index, repeated-miss issue planning, council reports, supervisor v6 planning visibility, and specialist/learning/autonomy v3 status dashboards.

## Implemented as target-repo scaffolds

- AI platform, provider abstraction, OpenAI-compatible/Ollama support, AI settings, RAG assistant, token/cost tracking dashboard, MCP diagnostics, NLAI capability interface, and pretty rendering.
- Product baseline features: in-app documentation center, settings area, onboarding tutorials, reporting dashboard, analytics metrics, feedback/support flow, diagnostics/status page, and visual design system.
- Tutorial/PDF/reporting/visualization expectations are scaffolded as specs and issue drafts for target repositories, not implemented as generic app runtime code.
- Architecture, UI/UX, testing, documentation, reporting, AI/NLAI, diagnostics, dependency, and security/privacy templates provide target-repo implementation plans. They are not generic runtime implementations.
- Product/AI/NLAI feature recipes can generate specs, metadata plans, issue drafts, templates, and bounded scaffolds. Stack-specific runtime files are generated only when the target stack is confidently detected and the recipe/capability permits it.
- Recognized-stack runtime shells are implemented for product, AI, NLAI, reporting, and Playwright evidence slices. They are shell/partial implementations unless tests and metadata prove complete behavior.
- Runtime evidence artifacts can prove local renderability only when launch profiles, dependencies, and Playwright/tooling already exist. Autospec plans/adopts missing tooling through local issues and does not install dependencies.
- Digital Twin surface maps, knowledge graph, and summary files are generated state under `.autospec/state/`. They are implemented by `scripts/autospec-build-digital-twin.sh`/`scripts/autospec-digital-twin.py`, but a fresh checkout must run the builder to refresh live state evidence.

## Validated by policy/rules only

- Design-pattern discipline, ADR expectations, dependency governance, modernization planning, migration discipline, Playwright viewport/screenshot expectations, visual/accessibility/performance/migration testing doctrine, reporting chart-selection standards, and documentation drift expectations.
- These are represented by structured rules/checks/gates and may generate backlog issues, but many require target-repository implementation.
- Doctrine audits add heuristic evidence collection and local issue drafts. A pass means Autospec found evidence; it is not a substitute for human review on high-risk product/runtime behavior.
- Visual polish and accessibility evidence audits are heuristic and evidence-based. They are not human design review, WCAG certification, or security/privacy approval.
- Documentation drift detection is implemented through the metadata drift validator. It is heuristic and focused on stale/missing metadata links, orphan docs/tests, and required Digital Twin files; it is not a semantic documentation correctness proof.
- Specialist agents are deterministic role/checklist/review systems, not independent LLM personas unless a future runtime explicitly connects them.
- Review quorum is an internal promotion gate and does not replace human review or approval.
- Learning ledger and memory index are repo-local; there is no cross-repo global learning service in this batch.

## Deferred beyond MVP

- Full AI assistant implementation in arbitrary target repositories.
- Full NLAI runtime implementation in arbitrary target repositories.
- Automatic dependency major upgrades.
- Automatic database migrations.
- Automatic auth/security behavior changes.
- Full visual UI generation.
- Guaranteed screenshot/PDF generation in arbitrary repositories without existing local tooling.
- Real external AI/provider simulation; AI/NLAI simulation is mock-only by default.
- Automatic application of policy proposals to `autospec-constitution` or `autospec-baselines`.
- Medium-risk code execution; medium-risk work is planned, decomposed, and routed to guidance instead.
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

## Platform-limited

- Reclaiming a lifecycle lease from a terminated-but-unreaped (zombie) holder is Linux-only. The
  conductor arms a subreaper, so an orphaned holder can stay unreaped for the life of the run;
  `process_is_terminated` reads the process state from procfs and returns `Ok(false)` on every
  other host, so macOS, FreeBSD, and Windows still wait for the holder's PID entry to disappear.
- Managed GitHub Project resolution, creation, marker verification, item reconciliation, and
  onboarding require a working authenticated `gh` CLI with Project read/write scope. Transient
  network, rate-limit, and ambiguous mutation failures remain retryable in the repo-local journal;
  missing executables, authentication/scope failures, malformed responses, and identity conflicts
  fail closed.
- Existing-repository discovery is intentionally incomplete and bounded. It starts only from
  explicit repositories, an allowlisted owner, or workspace paths, follows supported concrete
  metadata, and never expands beyond `repo_allowlist` or `discovery_max_repos`. Out-of-bound and
  inaccessible candidates are reported rather than indexed.
- Relationship inference is conservative. Deterministic repository metadata can create active
  dependency/blocking edges; ambiguous name-only relationships remain proposed and cannot affect
  execution until stronger evidence is recorded.
- Ordinary managed reconciliation is additive. It preserves human-managed Project content and has
  no deletion/pruning path for stale items, repositories, fields, or relationships.

## Requires human guidance

- Auth, authorization, permissions, privacy/security policy, billing/payments, migrations, data deletion, deployment/infrastructure, major dependency upgrades, framework migrations, public API breaking changes, and multi-service behavior.
- Worker v1/v2 remains bounded to docs/spec/metadata/test and low-risk code work unless a human explicitly narrows and approves the risk.
- Worker v3 recipe execution remains bounded by the worker capability registry, stack profiles, patch budgets, and verifier review.
- Worker v4 runtime feature execution remains opt-in through explicit feature invocation; supervisor runtime generation is disabled by default.
- Worker v5 evidence generation remains bounded to local artifacts. Confirm is required for process launches and user-facing artifact writes; missing tools produce adoption issues/specs instead of implicit installation.
- Autonomy v3 does not grant merge, approval, quorum bypass, verifier bypass, or automatic resume authority.

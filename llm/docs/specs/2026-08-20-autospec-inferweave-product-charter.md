# Autospec InferWeave product charter

## Status

Approved direction, 2026-08-20. This charter records the product boundary that follows the repository extraction. It is not itself decomposed into implementation issues; the target repository will carry staged design specs for the application, node control, and InferWeave protocol.

## Product outcome

`berlinguyinca/autospec-inferweave` is a private, standalone inference-node product and future InferWeave network client. It contains the node runtimes, outbound node agent, model artifact tooling, OpenAI-compatible data plane, application control plane, Angular frontend, MCP integration, operator documentation, tests, and measured hardware evidence currently spread through `autospec/llm` and related design documents.

The application is **AI-assistant first**. Its default route is Chat. Operations, Models, Nodes, MCP, Administration, Usage, Health, and Guide are secondary routes reached from the assistant shell. The first release has a fixed, reusable design system and fixed product routes. AI-generated dashboards, runtime-generated Angular templates, arbitrary widgets, and generated HTML/JavaScript are out of scope.

## Locked architecture

- Angular 22 standalone CSR application with Signals, Angular Router, CDK primitives, Vitest, and real-browser Playwright coverage.
- Fastify TypeScript application/control-plane service using the official MCP TypeScript SDK v2.
- Existing Python inference gateway retained as the data plane until a separately justified migration.
- PostgreSQL 18 is the only database. No SQLite mirror, queue, cache, or fallback database is permitted.
- OpenAI-compatible `/v1` remains stable. Application contracts live under versioned `/api/v1`; future network discovery begins at `/.well-known/inferweave-node` with an explicit `0.x` contract.
- MCP is a server-side integration adapter, not the node-management or InferWeave federation protocol.
- Model id selects weights, quantization, capabilities, and context profile. A deployment route `/u/<server>/v1` selects a server/GPU runtime. An individual GPU is selectable only when exposed as its own registered runtime.

## PostgreSQL-only state

PostgreSQL authoritatively stores identities, hashed API keys, roles, node enrollment, artifact catalog, model-delivery jobs, leases, audit events, MCP registry and tool policies, conversations, typed message parts, usage, and retained telemetry. Application roles are non-owner, non-superuser, and do not carry `BYPASSRLS`; tenant/user-owned data uses forced row-level security.

Ephemeral process memory may hold connection state, bounded request activity, SSE subscribers, and short-lived read caches. It is never a second durable authority. PostgreSQL unavailability fails closed for new authentication and administrative writes; already-admitted inference may finish, while non-authoritative telemetry may drop with an observable counter rather than block inference.

## Chat and MCP

Chat streams typed events: text, sanitized Markdown, code, reasoning status, tool proposal, approval request, tool result, resource, routing, usage, and error. The browser never receives provider or MCP credentials and never executes tools directly.

MCP servers are registered by administrators and connected by isolated server-side clients. Each tool has a schema, permission scope, risk level, output limit, timeout, audit policy, and approval rule. Unknown or stale tools are rejected. Tool output is untrusted and sanitized before model or browser use. Mutating or sensitive tools require explicit confirmation.

## Remote model delivery

Authenticated administrators may select a curated immutable artifact and a connected node in the frontend. The assistant may explain or prefill the request, but it cannot execute a download without explicit administrator confirmation.

Node management uses a dedicated outbound control protocol. It exposes narrow verbs such as `download-model`, `activate-model`, `cancel-job`, and `report-status`; it never accepts arbitrary shell, URLs, packages, or root commands. PostgreSQL stores the durable job and audit trail. Nodes enforce compatibility, disk budgets, allowlisted directories, immutable digest and signature verification, resumable staging, and atomic activation. Model bytes live in an artifact registry/object store, not PostgreSQL.

## Service documentation

The repository ships audience-specific Markdown documentation and a rendered in-app Guide. Documentation covers model/quantization selection, balanced versus pinned deployment routes, node enrollment, remote model delivery, MCP trust boundaries, PostgreSQL recovery, and future InferWeave protocol compatibility. Architecture and control-flow documents use Mermaid diagrams.

## Team personality

**Selected team: secure product platform.** Roles: product architect, Angular/product engineer, TypeScript control-plane engineer, Python data-plane maintainer, PostgreSQL/security engineer, node-agent/platform engineer, and test/QA engineer. This team fits because the feature combines a polished product, remote infrastructure control, authentication, database authority, and untrusted model/tool input.

Expected risks: credential or tool leakage, unsafe remote commands, RLS bypass, model-artifact substitution, UI sanitization failure, inference regressions, protocol drift, and observability that mislabels stale data as live.

### Review counter-team

**Counter-team: hostile operator and compatibility review.** Roles: security reviewer, SRE, accessibility reviewer, API compatibility reviewer, and local-LLM implementation reviewer. It challenges privilege escalation, broken recovery, inaccessible chat/tool flows, public-contract drift, and issue shapes too large for the intended cluster.

## Program decomposition

1. Repository extraction and independent CI.
2. PostgreSQL-only application/control-plane foundation and versioned contracts.
3. Angular assistant-first frontend, rich chat, and operations/service catalog.
4. MCP registry, tool execution, approval, and audit.
5. Admin model-delivery control plane and node-agent commands.
6. InferWeave node descriptor and capability negotiation.
7. No-mock QA, documentation, deployment, and release validation.

Each stage receives its own Autospec design, issues, dependency graph, and verification evidence in the target repository.

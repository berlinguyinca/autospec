# Pi Agent Spec and Handoff Protocol Design

**Date:** 2026-08-22
**Status:** Proposed for implementation
**Extends:** `docs/superpowers/specs/2026-08-21-unified-routing-foundation-design.md`

## 1. Purpose

AutoSpec will use Pi as a portable agent execution plane that can consult Claude Code and
Codex through explicitly allowlisted bridge extensions. Agents communicate only through
versioned, immutable specification and handoff artifacts. AutoSpec remains the control plane
for routing, issue claims, worktree isolation, validation, remote mutation, and merge authority.

The protocol converts product intent into an exact, repository-grounded specification before
an implementation model receives work. It also converts implementation results into a bounded
review handoff so an independent reviewer can verify claims without inheriting the planning or
implementation conversation.

## 2. Goals

1. Add versioned contracts for an approved specification, implementation handoff, and review
   handoff.
2. Let Pi invoke allowlisted `AskClaude` and `AskCodex` tools for bounded planning and review
   roles.
3. Keep each agent's output immutable and preserve complete artifact lineage.
4. Require repository evidence before a proposal becomes an approved specification.
5. Require zero unresolved material questions before implementation dispatch.
6. Preserve the current Pi adapter's extension-free behavior unless the operator explicitly
   enables a bridge profile.
7. Fail closed when a bridge is unavailable, returns malformed output, violates its assigned
   mode, or cannot satisfy independence requirements.
8. Preserve all existing routing fallbacks, model-tier rules, worktree isolation, implementation
   limits, guardian checks, closeout requirements, and merge gates.

## 3. Non-goals

- Agents do not conduct unstructured peer-to-peer conversations.
- Bridge agents do not claim issues, create or merge pull requests, alter queue ownership, or
  control the AutoSpec conductor.
- Version 1 does not provide arbitrary extension loading, arbitrary tool discovery, recursive
  delegation, or a persistent Pi RPC session.
- Version 1 does not make Claude Code or Codex a replacement for the deterministic issue linter,
  routing resolver, implementation linter, security scan, test suite, or merge gate.
- Version 1 does not weaken reviewer independence when no independent provider is available.

## 4. Design principles

### 4.1 Artifact protocol, not shared chat

Each phase consumes immutable files and produces a new immutable file. A consumer may cite an
input artifact but must never edit it. The canonical lineage is:

```text
intent
  -> planning proposal
  -> repository critique
  -> approved specification
  -> implementation handoff
  -> implementation closeout
  -> review handoff
  -> review verdict
```

Conversation history is never required to reproduce a downstream dispatch. Every prompt is
constructed from the validated artifact named in the dispatch record plus the repository's
normal `AGENTS.md` instructions.

### 4.2 AutoSpec retains authority

Pi and bridge agents may read, reason, edit inside an assigned worktree, and run allowed local
commands. Only AutoSpec may transition queue state, own a claim, push, create or edit a pull
request, decide merge readiness, or merge.

### 4.3 Explicit trust and independence

Extension loading is deny-by-default. The operator enables one named bridge profile containing
an exact extension allowlist. Every agent result records harness, bridge, provider family, model,
session isolation, access mode, and input artifact digest.

Provider independence and session independence are distinct. A second Claude session is
session-independent but not provider-independent from a Claude proposer. A review that requires
provider independence fails closed when only same-provider candidates remain.

## 5. Versioned artifacts

All artifacts use JSON Schema draft 2020-12, reject unknown properties, use repository-relative
paths, and contain no credentials. Each carries `version: 1`, an artifact ID, creation timestamp,
repository identity, source artifact digests, and producer identity.

### 5.1 Approved specification: `autospec-spec-v1`

The approved specification contains:

- one concrete goal;
- explicit non-goals;
- constraints and invariants;
- affected existing and proposed paths and symbols;
- decisions and rejected alternatives;
- machine-checkable acceptance criteria;
- required test tiers and exactly one primary smoke command;
- risks and mitigations;
- planning evidence from the intent planner and repository critic;
- material questions, which must be an empty array when status is `approved`;
- approval status: `proposal`, `needs_revision`, or `approved`.

An approved artifact must cite both an intent-planning result and a repository-grounding result.
The two results may come from the same harness but must use separate isolated sessions. A policy
may additionally require distinct provider families.

### 5.2 Implementation handoff: `autospec-implementation-handoff-v1`

The implementation handoff contains only the context necessary to implement one bounded issue:

- approved specification ID and digest;
- issue number, repository, branch, worktree, and claim generation;
- exact allowed read and write paths;
- issue goal and selected acceptance criteria;
- constraints and invariants relevant to the issue;
- expected symbols and interfaces;
- required tests and primary smoke command;
- maximum tool calls and self-review iterations;
- cumulative retry findings;
- expected closeout artifact path and schema;
- producer and selected implementation route.

Generation fails when the source specification is not approved, has material questions, does not
identify a primary smoke command, or cannot assign a non-empty bounded write scope.

### 5.3 Review handoff: `autospec-review-handoff-v1`

The review handoff contains:

- implementation handoff ID and digest;
- exact base and head commits;
- scoped changed paths;
- closeout claims and cited proof artifacts;
- acceptance criteria to verify;
- commands the reviewer may rerun;
- required reviewer independence;
- deterministic linter and test summaries;
- expected structured verdict path and schema.

Generation fails unless the implementation closeout exists, parses, cites the same handoff, and
the actual changed paths remain inside the handoff's write scope.

### 5.4 Agent result envelope: `autospec-agent-handoff-result-v1`

Every planning or review bridge returns one common result envelope:

- role: `intent_planner`, `repository_critic`, `implementation_advisor`, or `reviewer`;
- status: `pass`, `needs_revision`, `blocked`, or `error`;
- findings with severity, claim, evidence paths, and confidence;
- proposed artifact body for the next phase when applicable;
- producer provenance;
- input artifact digests;
- commands and tools used;
- usage totals;
- unresolved questions;
- error category when status is `error`.

Free-form bridge output is diagnostic only and cannot advance the workflow.

## 6. Pi bridge profile

The existing Pi adapter continues to pass `--no-extensions`, `--no-skills`, and
`--no-prompt-templates` when no bridge profile is selected. A bridge-enabled dispatch uses a
fresh private `PI_CODING_AGENT_DIR` containing only generated configuration and an exact package
allowlist.

Repository configuration selects logical tools rather than package implementation details:

```yaml
pi_bridge:
  enabled: true
  extensions:
    ask_claude:
      package: "npm:pi-claude-bridge@0.7.0"
      tool: "AskClaude"
    ask_codex:
      package: "npm:@estebanforge/pi-ask-codex@1.0.3"
      tool: "AskCodex"
  policy:
    max_parallel: 2
    recursive_delegation: false
    require_isolated_planning_sessions: true
```

The installed package inventory, not the network, resolves the pinned packages during an
AutoSpec run. AutoSpec never installs packages automatically in an autonomous dispatch. Missing
or mismatched packages produce `HANDOFF_BRIDGE_UNAVAILABLE`.

### 6.1 Access mapping

| Role | Claude mode | Codex sandbox | Writes |
|---|---|---|---|
| Intent planner | `none` or `read` | `read-only` | artifact output only |
| Repository critic | `read` | `read-only` | artifact output only |
| Implementation advisor | `read` | `read-only` | artifact output only |
| Implementer | `full` | `workspace-write` | assigned worktree scope |
| Reviewer | `read` | `read-only` | verdict artifact only |

`danger-full-access` is never emitted by the version 1 bridge adapter. Implementation remains a
normal routed harness dispatch unless an explicit later policy authorizes bridge implementation.

## 7. Orchestration flow

### 7.1 Planning

1. AutoSpec writes a normalized intent artifact.
2. Pi asks the intent planner for a proposal in the common result envelope.
3. AutoSpec validates and stores the proposal without modifying the intent.
4. Pi asks a repository critic in an isolated session to inspect the proposal and repository.
5. AutoSpec validates and stores the critique.
6. A deterministic reconciler accepts the proposal only when there are no blocking conflicts and
   all cited existing paths and symbols resolve. Otherwise it writes `needs_revision` with exact
   findings for another bounded planning pass.
7. The existing issue-quality linter validates the rendered specification and decomposition.

The reconciler does not invent resolutions. Conflicting load-bearing recommendations require a
new planner result that explicitly accepts one alternative and rejects the other with rationale.

### 7.2 Implementation

1. AutoSpec decomposes the approved specification into issue-sized work.
2. It creates one implementation handoff per claimed issue.
3. The existing unified router selects the implementation harness and inference route.
4. The implementation agent receives only the validated handoff and repository instructions.
5. The existing implementation linter, security scan, tests, closeout contract, and retry limits
   remain authoritative.

### 7.3 Review

1. AutoSpec derives a review handoff from the implementation handoff, git commits, deterministic
   checks, and closeout report.
2. The router selects a reviewer satisfying the declared independence policy.
3. Pi may invoke `AskClaude` or `AskCodex` in an isolated read-only session.
4. AutoSpec validates the structured verdict and independently re-reads cited proof.
5. Only the existing merge gate can advance the PR.

## 8. Error handling and fallback

Stable error categories are:

- `HANDOFF_SCHEMA_INVALID`
- `HANDOFF_LINEAGE_MISMATCH`
- `HANDOFF_UNRESOLVED_MATERIAL_QUESTION`
- `HANDOFF_SCOPE_INVALID`
- `HANDOFF_BRIDGE_DISABLED`
- `HANDOFF_BRIDGE_UNAVAILABLE`
- `HANDOFF_TOOL_UNAVAILABLE`
- `HANDOFF_AGENT_OUTPUT_INVALID`
- `HANDOFF_AGENT_FAILED`
- `HANDOFF_INDEPENDENCE_UNSATISFIED`
- `HANDOFF_EVIDENCE_INSUFFICIENT`

Planning and review failures do not silently become approvals. For optional advisory calls, the
workflow records the failure and continues through the pre-existing non-bridge path. For required
dual-agent planning or independent review, it stops before implementation or merge respectively.

The global rollback is state-free: disable `pi_bridge.enabled` or remove the bridge section. The
current extension-free Pi adapter and legacy routing behavior then remain unchanged.

## 9. Security and privacy

- Extension package names and versions must match the installed allowlist.
- AutoSpec does not run package installers during autonomous execution.
- Credentials stay in environment variables and never enter artifacts, prompts, or argv.
- Generated Pi configuration lives in a private temporary directory and is deleted after dispatch.
- All file paths in artifacts are repository-relative; traversal and symlink escapes fail closed.
- Read-only roles cannot request write-capable bridge modes.
- The adapter disables nested delegation for children and caps bridge calls at two per planning
  cycle.
- Agent output is untrusted until schema, lineage, scope, and evidence validation succeeds.

## 10. Interfaces and files

New focused surfaces:

- `schemas/autospec-spec-v1.schema.json`
- `schemas/autospec-implementation-handoff-v1.schema.json`
- `schemas/autospec-review-handoff-v1.schema.json`
- `schemas/autospec-agent-handoff-result-v1.schema.json`
- `scripts/autospec-handoff.py` — validate, reconcile, and derive artifacts.
- `scripts/autospec-pi-bridge-dispatch.py` — run one allowlisted bridge role and normalize output.
- `examples/pi-agent-handoff.yml` — operator configuration example.
- `tests/pi-agent-handoff/` — hermetic schema, lineage, adapter, fallback, and integration tests.

Existing surfaces to extend:

- `scripts/autospec-pi-dispatch.py` — preserve extension-free default and accept a validated bridge
  profile only for bridge dispatches.
- `skills/autospec-run/{SKILL.md,opencode/agent.md,codex/prompt.md}` — implementation and review
  handoff requirements in lock-step.
- `skills/autospec-define/{SKILL.md,opencode/agent.md,codex/prompt.md}` — dual-agent planning and
  approved-spec gate in lock-step.
- `skills/autospec-run/install.sh` and `skills/autospec-define/install.sh` — install runtime files
  and schemas.
- `docs/CONFIG_REFERENCE.md`, `docs/API_REFERENCE.md`, and `docs/USER_MANUAL.md`.

CLI surface:

```text
python3 scripts/autospec-handoff.py validate --kind spec|implementation|review|result --input FILE
python3 scripts/autospec-handoff.py reconcile-spec --proposal FILE --critique FILE --output FILE
python3 scripts/autospec-handoff.py implementation --spec FILE --issue FILE --output FILE
python3 scripts/autospec-handoff.py review --implementation FILE --closeout FILE \
  --base COMMIT --head COMMIT --output FILE
python3 scripts/autospec-pi-bridge-dispatch.py --config FILE --role ROLE \
  --input FILE --output FILE
```

## 11. Testing

Tests are hermetic and use temporary repositories plus executable Pi, Claude, and Codex stubs.
They never require network access, installed community extensions, subscriptions, or credentials.

Required proof:

1. Every valid example validates against its schema; unknown fields and malformed provenance fail.
2. An approved spec cannot contain unresolved material questions.
3. Reconciliation preserves immutable inputs and records both source digests.
4. A nonexistent cited existing path or symbol prevents approval.
5. Implementation handoff generation rejects unapproved specs and empty write scopes.
6. Review handoff generation rejects changed paths outside the implementation scope and closeout
   lineage mismatches.
7. The Pi bridge adapter exposes only the configured logical tool, selects the required read/write
   mode, passes no secret in argv, and rejects malformed JSONL.
8. Missing bridge packages fail with `HANDOFF_BRIDGE_UNAVAILABLE` without changing existing Pi
   dispatch behavior.
9. Same-provider reviewers fail a provider-independence requirement.
10. Lock-step skill bodies, installer manifests, schema presence, shell syntax, Python tests,
    routing-foundation tests, and `cargo test --workspace` pass.

## 12. Acceptance criteria

- AutoSpec can produce and validate all four version 1 artifacts through documented commands.
- A hermetic end-to-end test turns intent-planner and repository-critic stub results into an
  approved spec, derives an implementation handoff, derives a review handoff from a closeout, and
  validates an independent reviewer result.
- Pi can invoke configured Claude and Codex bridge tools in separate isolated sessions and return
  schema-valid results.
- No implementation begins from a spec with unresolved material questions or invalid repository
  evidence.
- No review advances from a lineage mismatch, scope violation, malformed result, or unsatisfied
  independence requirement.
- With bridge configuration absent or disabled, the byte-observable existing Pi invocation and
  unified routing fallback remain unchanged.
- All multi-harness skill bodies remain lock-step and all repository validation required by
  `AGENTS.md` passes.

## 13. Rollout

1. Ship schemas, artifact tooling, and hermetic validation without enabling bridge dispatch.
2. Enable read-only planning and repository critique behind `pi_bridge.enabled`.
3. Run independent review in shadow mode and record disagreements without affecting merges.
4. Promote independent review to a required gate after measured evidence shows stable output.
5. Consider bridge-based implementation and persistent Pi RPC only in later specifications.

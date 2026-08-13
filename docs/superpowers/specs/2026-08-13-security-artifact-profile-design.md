# Security-Critical Spec and Issue Generation Design

**Date:** 2026-08-13  
**Status:** Approved direction; implementation planning pending written-spec review  
**Scope:** `/autospec` and `/autospec-define` investigation, design, decomposition, and pre-implementation validation

## Purpose

Autospec must turn security- and data-sensitive feature requests into evidence-backed design specs, concise epics, and dependency-ordered implementation issues without compressing away load-bearing controls. A request such as a read-only SQL capability for an LLM-facing production service must preserve threat mitigations, database authority, operational prerequisites, adversarial tests, residual risks, and unresolved facts while still producing child issues small enough for the existing implementation loop.

The requested long-form artifact is a source brief or design spec, not a single `auto-implement` child. Autospec will keep the rich design in `docs/specs/`, render a concise epic that links to it, and generate children that each remain within the existing 400-word, 30-outline-line, and three-logical-unit limits.

## Evidence and current limitations

- Phase 1 already requires real-system investigation for remote databases and servers, but its findings are returned as prose and are not preserved in a structured handoff.
- Phase 2 asks about architecture, API shape, data model, error handling, and testing, but has no conditional completeness contract for threat models, authority layers, safety priorities, blocking prerequisites, or residual risks.
- Phase 3 produces compact child mini-specs and checks `Produces`, `Consumes`, and `Covers`, but does not prove threat-to-control, control-to-test, or spec-to-child coverage.
- `scripts/gen-issue-skeleton.sh` provides a deterministic issue renderer, but its schema trails the live Phase 3 contract and the main decomposition flow does not use it as the authoritative render path.
- `scripts/lint-issue.sh` enforces useful formatting and size constraints, but dependency semantics and several advertised requirements are incomplete.

## Approaches considered

### Prompt-only security guidance

Add a security checklist to Phase 2 and Phase 3 prompts. This is the smallest change, but it leaves completeness dependent on model attention and gives validators no structured facts to inspect. Rejected because the failure mode is silent loss of a load-bearing control.

### Fully deterministic templates

Require every feature to use one large schema and render all specs and issues mechanically. This improves consistency but makes routine features noisy and forces nuanced design decisions into brittle rules. Rejected because semantic research and decomposition remain model-suited work.

### Conditional structured profile with deterministic gates

Use the LLM for investigation, design synthesis, and issue boundaries; require a structured intermediate artifact only when deterministic signals select a high-risk profile; render stable Markdown from that artifact; validate coverage and prerequisites before queueing children. Selected because it preserves judgment while making omissions observable and fail-closed.

## Feature profile selection

Phase 1 emits a `feature_profile` decision alongside its evidence. The initial profile is `security_database`; no general plugin system is introduced in this change.

Select `security_database` when the request or repository evidence includes any of:

- database roles, grants, migrations, raw or generated queries;
- production or replica data access;
- authorization, credentials, secrets, or subject-level data;
- untrusted or LLM-authored input reaching a database or privileged service;
- runtime controls intended to prevent data loss, exposure, or availability damage.

If selection is uncertain, use the profile. False-positive rigor is preferable to silently omitting a database safety invariant. Ordinary features retain the existing Phase 2 and Phase 3 flow.

## Structured artifact

For `security_database`, Phase 2 must create a sidecar at:

```text
.autospec/spec-artifacts/<spec-slug>.security-database.yml
```

The tracked Markdown spec remains the reviewer-facing source of truth. The sidecar is a deterministic validation and rendering input committed with the spec.

Required top-level fields:

```yaml
schema: autospec.security_database.v1
spec_path: docs/specs/YYYY-MM-DD-topic-design.md
feature_profile: security_database
evidence: []
facts: []
assumptions: []
priority_order: []
blocking_prerequisites: []
threats: []
controls: []
negative_tests: []
residual_risks: []
issues: []
```

### Evidence and fact status

Every external or repository-derived design claim is represented as one of:

- `verified`: directly checked during Phase 1, with a path, command, URL, or query summary;
- `assumed`: plausible but not verified and prohibited from becoming an implementation instruction;
- `blocking`: unresolved and must prevent affected child issues from receiving `auto-implement`;
- `accepted`: a deliberate risk or product decision with rationale.

Secrets and sensitive query results are never stored in the sidecar. Evidence records contain summaries and reproducible safe probes only.

### Controls

Each threat has a stable ID. Each control references that threat ID and records its mitigation, enforcement owner, authority strength, failure consequence, and verification IDs. Authority strength is one of `advisory`, `runtime`, or `authoritative`.

```yaml
controls:
  - id: T1
    threat_id: TH1
    threat: data modification
    mitigation: restricted database role holds SELECT only
    owner: database
    authority: authoritative
    failure_consequence: data loss
    verification: [GT1, GT2, GT3, GT4]
```

The profile requires at least one authoritative control for every threat whose failure consequence is data loss or unauthorized disclosure. Application validation alone cannot satisfy that rule.

### Blocking prerequisites

Prerequisites carry an ID, a safe verification command or evidence reference, and the issue IDs they gate. A blocked consumer may be filed for visibility, but it receives `autospec:blocked-prerequisite` instead of `auto-implement`. Phase 3.5 may promote it only after the prerequisite has verified evidence.

Open questions are therefore classified rather than copied indiscriminately into implementation issues. For example, actual table names and replica availability are blocking; an already chosen identifier precedence rule is a resolved decision.

### Issue mapping

Each planned child declares:

- `produces`, `consumes`, and spec section anchors;
- exact prerequisite IDs and sibling dependency keys;
- control IDs it implements;
- verification IDs it supplies;
- whether it must remain atomic with a coupled validation artifact.

The issue list is an intermediate plan, not the final Markdown body. Phase 3 resolves sibling keys to GitHub issue numbers only after the graph validates.

## Generated artifacts

The pipeline produces three distinct artifact classes:

1. **Design spec:** unrestricted enough to preserve the complete problem, constraints, priority order, architecture, threat model, testing matrix, residual risks, and decisions.
2. **Epic:** a concise summary linking the spec, stating delivery tiers, blocking prerequisites, and a checklist of children. It never carries `auto-implement`.
3. **Implementation child:** the existing small-LLM mini-spec, extended with compact `Controls covered`, `Evidence consumed`, and `Prerequisites` sections.

The supplied read-only query example would decompose approximately into:

1. Verify schema, identifier storage, and read-replica availability.
2. Define the shared identification predicate and parameterized query repository.
3. Implement the curated Tier 1 tools and response-envelope rows.
4. Provision and verify the restricted database role and isolated pool.
5. Implement the Tier 2 AST statement policy.
6. Add cost, row, byte, rate, timeout, audit, and runtime-disable controls.
7. Add parser-negative, grant-bypass, and cross-tier integration tests.

Tier 2 issues consume the verified replica and role prerequisites. If no replica exists, they remain blocked without weakening Tier 1.

## Deterministic validators

Add a focused validator for the sidecar and generated issue set. It fails with stable `RULE_ID: description` findings suitable for the existing cumulative retry loop.

Required rules:

- `PROFILE_SCHEMA_INVALID`: required fields or enum values are invalid.
- `EVIDENCE_UNRESOLVED`: an implementation instruction depends on `assumed` evidence.
- `BLOCKING_PREREQUISITE_QUEUED`: a blocked issue carries `auto-implement`.
- `THREAT_WITHOUT_CONTROL`: a threat has no mitigation.
- `AUTHORITATIVE_CONTROL_MISSING`: a data-loss or disclosure threat lacks a database/platform authority.
- `CONTROL_WITHOUT_TEST`: a control has no verification ID.
- `NEGATIVE_TEST_UNOWNED`: an adversarial test maps to no child.
- `SPEC_SECTION_UNCOVERED`: a required implementation section maps to no child.
- `DEPENDENCY_UNKNOWN`: an issue consumes an unknown sibling or prerequisite.
- `DEPENDENCY_CYCLE`: the planned issue graph is cyclic.
- `ATOMIC_CONTRACT_SPLIT`: coupled implementation and validation artifacts were split across incompatible children.

The validator never attempts to judge whether a mitigation is semantically sufficient. A Tier-A semantic reviewer performs that judgment after deterministic validation and before filing.

## Existing issue-lint repairs

This work also aligns `scripts/lint-issue.sh` with its documented contract:

- reject acceptance criteria that lack a path, backtick span, integer, or regex token;
- require the primary smoke-test section and exactly one executable line;
- enforce one Goal sentence rather than accepting two;
- require a Dependencies section and validate graph semantics in the profile validator;
- exclude marker-bounded generated metadata from the authored 400-word budget.

These repairs apply to all generated children, not only the new profile, and require regression fixtures before implementation edits.

## Pipeline integration

### Phase 1

Return the profile decision, safe evidence records, verified facts, assumptions, and blocking unknowns. Do not put credentials, sensitive rows, or unrestricted production output into the handoff.

### Phase 2

When the profile is active, add explicit sections for constraints, priority order, authoritative invariants, threat model, negative tests, operational disablement, residual risks, and classified open questions. Write the Markdown spec and sidecar together, then validate both before opening the spec PR.

### Phase 3

Generate the intermediate issue graph, validate it, run the Tier-A semantic coverage review, and render issue bodies deterministically. Existing lint and safety retries still run on the final rendered bodies. The renderer becomes the authoritative path rather than optional tooling.

### Phase 3.5 and runtime

Phase 3.5 verifies prerequisite state before preserving `auto-implement`. `autospec-run` refuses any issue carrying `autospec:blocked-prerequisite`, missing required safety metadata, or referencing unresolved blocking evidence.

### Cross-issue review

After rendering but before queue promotion, run one portfolio review against the spec and sidecar. It asks whether deleting the application validator could permit data loss, whether each control has an independent backstop where required, and whether the union of child tests proves the epic acceptance criteria. Findings return to Phase 3 as cumulative correction context.

## Failure handling

- Malformed or missing profile sidecar: fail closed before the spec PR.
- Unsupported schema version: fail closed with the observed and supported versions.
- Unverified remote prerequisite: preserve the epic and blocked issue, but do not queue the consumer.
- Renderer or validator unavailable: do not fall back to free-form issue filing for an active security profile.
- Semantic reviewer unavailable after normal tier fallback: preserve draft artifacts locally and stop before GitHub issue creation.
- Final Markdown fails ordinary issue lint or safety lint after five retries: skip that child and report the uncovered spec/control IDs.

## Testing

Validation uses repository-native shell/Bats fixtures and no new dependency.

Required fixture families:

- minimal valid `security_database` spec artifact;
- missing authoritative database control;
- control without verification;
- unresolved assumption consumed by a child;
- blocked prerequisite incorrectly queued;
- unknown and cyclic issue dependencies;
- uncovered spec section and negative test;
- atomic implementation/test contract split;
- deterministic rendering golden containing evidence, controls, and prerequisites;
- ordinary non-security issue proving the conditional profile does not add sections;
- regressions for AC token enforcement, required smoke test, one-sentence Goal, and metadata-excluded word count.

Focused tests run first. The repository's fast validator and launch-readiness validator run after every task; full `autospec validate` runs before completion. The existing macOS Rust compilation failure is baseline evidence and not part of this design's scope.

## Non-goals

- Implementing an LCB MCP server or any database query feature.
- Creating a general-purpose policy language or arbitrary feature-profile plugin system.
- Replacing Tier-A semantic research or design judgment with regexes.
- Relaxing current issue size, safety, test, or worktree gates.
- Storing credentials, raw sensitive data, or unrestricted query output in artifacts.
- Automatically resolving infrastructure questions without safe read-only evidence.

## Compatibility and rollout

The profile is additive. Existing specs without `feature_profile: security_database` retain their present workflow. The renderer accepts the current minimal schema during migration, while the Autospec flows begin emitting the extended schema. Adapter bodies remain lock-step through `scripts/derive-trio.sh`; generated goldens are regenerated in the same commit as each canonical `SKILL.md` edit.

Rollout is fail-closed only for newly selected security profiles. Existing queued issues are not retroactively blocked unless `/autospec-classify` is explicitly run against them.

## Acceptance criteria

- A security/database request produces a tracked design spec, structured sidecar, epic, and dependency-ordered child issues.
- Blocking prerequisites cannot retain or receive `auto-implement`.
- Every data-loss or disclosure threat maps to a control with an authoritative database or platform enforcement layer.
- Every control maps to at least one verification, including parser-bypass grant tests where database authority is claimed.
- Every required spec section, negative test, and control is owned by at least one child.
- Generated children remain within existing size limits and link back to exact spec sections.
- Ordinary feature generation is unchanged when the profile is not selected.
- Final issue bodies pass the repaired issue lint and existing safety lint before filing.
- The main Autospec flows invoke the deterministic renderer and profile validator rather than treating them as optional documentation tools.
- No new dependency is introduced, and all multi-harness skill bodies remain lock-step.

## Likely hidden failure

A syntactically complete sidecar can still encode a weak mitigation, such as naming an application parser `authoritative` when the database grants remain broad. The deterministic validator cannot prove semantic adequacy; the Tier-A portfolio review and explicit authority-owner field are load-bearing defenses against that misclassification.

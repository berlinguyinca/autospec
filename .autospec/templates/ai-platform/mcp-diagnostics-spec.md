# MCP Diagnostics Spec

Define MCP registry diagnostics, server health, read-only defaults, mutation approvals, secret references, and audit logging.

## Purpose

Describe the target-repository capability and the user problem it solves.

## App-type applicability

Applies to web, internal-tool, AI-platform, analytics, and documentation-heavy apps when the matching baseline profile or rule requires it.

## Architecture recommendation

Add the smallest coherent slice first: metadata, settings/config boundary, implementation contract, validation evidence, and follow-up issue links.

## UI expectations

If user-facing, include responsive mobile/tablet/desktop behavior, empty/loading/error states, accessible labels, keyboard/touch behavior, and clear human-readable output.

## Settings/config expectations

Use explicit config keys and secret references only. Do not include production secret values in code, docs, metadata, or reports.

## Tests required

Add focused unit/integration tests for logic boundaries and regression coverage for generated metadata or reports.

## Playwright expectations

For web UI, include viewport coverage, screenshots where useful, console/network capture for failures, and accessibility checks when applicable.

## Docs/tutorial expectations

Update repo docs, in-app docs, tutorials, and RAG-ready source material when the feature changes user-visible behavior.

## Security/privacy notes

Document permission boundaries, audit logging, retention/export/delete behavior where relevant, and escalation criteria for auth/security review.

## Token/cost tracking

For AI-related capabilities, track token usage, model/provider, cost estimate, user/team ownership, quota/budget state, and redacted error evidence.

## Acceptance criteria

- [ ] Capability requirements are documented with source rule IDs where available.
- [ ] Tests and validation commands are listed.
- [ ] Metadata files expected to change are listed.
- [ ] Risk and worker eligibility are stated.

## Validation commands

```bash
bash scripts/autospec-check-rules.sh
bash scripts/autospec-mvp-status.sh
```

## Metadata files expected to change

- `.autospec/state/capability-registry.json`
- `.autospec/state/digital-twin.json`
- `.autospec/reports/rule-check-results.json`

## Worker eligibility/risk notes

Worker v1/v2 may handle docs, specs, metadata, tests, or bounded low-risk helper changes. High-risk auth, migration, dependency upgrade, deployment, or security behavior requires stuck/guidance.

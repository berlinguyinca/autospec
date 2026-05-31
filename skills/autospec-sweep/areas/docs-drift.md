# Sweep area: docs-drift

Detect drift between user-facing docs and implemented behavior.

## Researcher

Reuses the adapter doc-drift dogfood scanner:

- `scripts/dogfood-adapter-doc-drift.sh`

Reads tracked docs (`README.md`, `docs/**`, `AGENTS.md`, harness adapter prose),
compares declared invocations/flags/behaviors against the codebase, and emits
one proposal per stale or contradicting doc.

## Output

JSON to stdout matching the autospec-explore research-cycle contract:

```json
{ "source": "docs-drift", "proposals": [ ... ] }
```

If the upstream dogfood scanner does not emit research-cycle JSON, the
orchestrator wraps its findings into the contract before aggregation.

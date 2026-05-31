# Sweep area: spec-vs-code-drift

Detect drift between tracked acceptance criteria and implementation.

## Researcher

Reuses the autospec-explore deterministic researcher:

- `scripts/explore-research/spec-vs-code.sh`

Walks `docs/specs/**.md`, extracts unchecked acceptance criteria, greps the repo
for matching identifiers, and emits one proposal per unmatched AC.

## Output

JSON to stdout matching the autospec-explore research-cycle contract:

```json
{ "source": "spec-vs-code", "proposals": [ ... ] }
```

The sweep orchestrator aggregates these proposals into the sweep report and
hands them to `autospec-review` for filing as `auto-implement` issues.

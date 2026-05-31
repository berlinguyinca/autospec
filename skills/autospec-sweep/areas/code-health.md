# Sweep area: code-health

Surface TODO/FIXME markers, dead code, and low-coverage hot spots.

## Researcher

Reuses the autospec-explore deterministic researcher:

- `scripts/explore-research/codebase-signals.sh`

Scans tracked source for `TODO`/`FIXME`/`XXX` comments and other code-health
signals, and emits one proposal per signal cluster.

## Output

JSON to stdout matching the autospec-explore research-cycle contract:

```json
{ "source": "codebase-signals", "proposals": [ ... ] }
```

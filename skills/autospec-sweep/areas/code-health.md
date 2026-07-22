# Sweep area: code-health

Surface TODO/FIXME markers, dead code, and low-coverage hot spots.

## Researcher

Reuses the autospec-explore deterministic researcher:

- `scripts/explore-research/codebase-signals.sh`

Scans tracked source for `TODO`/`FIXME`/`XXX` comments and other code-health
signals, and emits one proposal per signal cluster.

## Three-lens review

Code-health findings are reviewed in bounded chunks and through three lenses:

- **Sentinel (security):** finds security footguns and assigns each finding a
  `Critical`, `High`, `Medium`, or `Low` severity.
- **Optimizer (performance):** finds measurable performance regressions and
  assigns the same severity levels. This lens is opt-in through the
  `performance` check.
- **Architect (maintainability):** finds excessive complexity, dead code, and
  duplication, with a `Critical`, `High`, `Medium`, or `Low` severity.

The map-and-partition step walks repository modules (falling back to the
largest directories) and partitions them into review-sized chunks using the
same context-budget sizing as `autospec-run`: `ctx:32k`, `ctx:64k`, or
`ctx:120k`. Findings are deduplicated across chunks and lenses, then grouped
by module and sorted from highest to lowest severity. Critical and High
findings remain eligible for the sweep `file_issues` and
`route_fixes_via_autospec_run` loop.

The `continuous_improvement.code.checks` configuration maps as follows:

| Check | Lens |
| --- | --- |
| `security_footguns` | Sentinel |
| `complexity` | Architect |
| `dead_code` | Architect |
| `duplication` | Architect |
| `performance` | Optimizer (opt-in) |

## Output

JSON to stdout matching the autospec-explore research-cycle contract:

```json
{ "source": "codebase-signals", "proposals": [ ... ] }
```

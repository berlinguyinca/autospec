# Sweep area: dependency-health

Detect outdated dependencies and best-effort security advisories.

## Researcher

New deterministic researcher, shared with autospec-explore (extends the
6-researcher set to 7):

- `scripts/explore-research/dependency-health.sh`

Detects dependency manifests in the repo (`package.json`, `requirements.txt`,
`go.mod`, `Cargo.toml`, `Gemfile`, `pyproject.toml`) and runs the harness-aware
outdated-check (`npm outdated --json`, `pip list --outdated --format=json`,
`go list -m -u all`, `cargo outdated`, etc.) when the toolchain is available.

The researcher is best-effort and harness-aware: missing toolchains produce
zero proposals rather than failing.

## Output

JSON to stdout matching the autospec-explore research-cycle contract:

```json
{ "source": "dependency-health", "proposals": [ ... ] }
```

# Autospec example configs

This directory ships two reference YAML files that downstream skills consume
when they need a starting point for user configuration. They live in the repo
so contributors can read the schema in source, and so the install flow can
seed `~/.autospec/` for new users on first run.

## Overview

| File                | Consumed by                                                     | Purpose                                                                  |
|---------------------|-----------------------------------------------------------------|--------------------------------------------------------------------------|
| `model-profiles.yml`| [`skills/autospec-run/SKILL.md`](../skills/autospec-run/SKILL.md)        | Maps named model profiles to ctx/reasoning budgets used by Phase 4.       |
| `project-map.yml`   | [`skills/autospec-classify/SKILL.md`](../skills/autospec-classify/SKILL.md) | Maps `ctx:*` / `reasoning:*` labels to GitHub Projects boards / swimlanes. |
| `fab.yml`           | [`skills/autospec-fab/SKILL.md`](../skills/autospec-fab/SKILL.md)        | Starter `.autospec/fab.yml` — generator, STL roots, printer profile, and the per-model gate list. |

## model-profiles.yml

Two-profile sample shipped here:

```yaml
claude-sonnet-cloud:
  model: claude-sonnet-5
  ctx: 200000
  reasoning: medium
  allowed: ctx:small,ctx:medium,reasoning:low,reasoning:medium

qwen3-6-35b-a3b-laptop:
  model: qwen3.6:35b-a3b
  ctx: 32000
  reasoning: low
  allowed: ctx:small,reasoning:low
```

Field reference (per profile entry):

- `model` — identifier the harness resolves at dispatch time.
- `ctx` — context-window budget (tokens) the profile is rated for.
- `reasoning` — reasoning tier hint: `low`, `medium`, or `high`.
- `allowed` — comma-separated list of issue labels the profile may pick up.
  Phase 4 dispatch filters issues by these labels before scheduling.

`/autospec-run --profile <name>` selects exactly one profile from this file.

## project-map.yml

Sample header:

```yaml
ctx:small:
  project: null
  board: small-context

reasoning:low:
  project: null
  board: mechanical
```

Field reference (per label key):

- `project` — GitHub Projects v2 project number, or `null` when unset.
- `board` — human-readable swimlane / board name.

Known label keys (per spec §4.3):

- `ctx:small` / `ctx:medium` / `ctx:large` — context-window classes.
- `reasoning:low` / `reasoning:medium` / `reasoning:high` — reasoning-effort tiers.

`/autospec-classify` looks up each classified issue's labels in this map and
moves the row onto the matching board after Phase 3.5 labeling completes.

## fab.yml

Starter `.autospec/fab.yml` for a CAD-as-code repo opting in to `/autospec-fab`.
It carries the `generator` regen command, `stl_roots`, the FDM `printer` profile,
an optional `metadata_sidecar` template, and the `models[]` gate list
(`name` / `stl` / `metadata` / `printable` / `load_critical` / `flow_critical`).

```yaml
generator: "rm -rf build && .venv/bin/python src/generate.py"
stl_roots:
  - "build/stls/manifolds/"
printer:
  nozzle_width: 0.4
  layer_height: 0.2
  min_perimeters: 3
  max_overhang_deg: 45
metadata_sidecar: "manifolds/{name}/metadata.json"
models:
  - name: "inlet_manifold"
    stl: "build/stls/manifolds/inlet_manifold.stl"
    metadata: "manifolds/inlet_manifold/metadata.json"
    printable: true
    load_critical: true
    flow_critical: false
```

Required keys in `--validate` mode: `generator` (non-empty) and `models`
(non-empty list). All other keys default at load time. Each model's `MODELDIR`
may also hold optional stage sidecars (`circuit.json`, `duct.json`,
`printer.json`, `load.json`, `flow.json`, `baseline.json`) that the release-gate
engine threads into the matching stage only when the file exists. See
[`skills/autospec-fab/README.md`](../skills/autospec-fab/README.md) for the full
contract and the loader CLI
([`load-fab-config.sh`](../skills/autospec-fab/scripts/load-fab-config.sh)).

## How skills consume these

- `/autospec-run` reads `~/.autospec/model-profiles.yml` (or this file as a
  fallback / seed) when resolving `--profile <name>`.
- `/autospec-classify` reads `~/.autospec/project-map.yml` (or this file as a
  fallback / seed) when routing classified issues onto Projects boards.

## Auto-init behavior

Skills MAY copy a file from this `examples/` directory to `~/.autospec/<file>`
on first run when the user-level config is missing. This keeps onboarding
zero-touch — the user gets a working default they can edit in place, and the
shipped sample stays read-only inside the repo.

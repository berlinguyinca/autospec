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

## model-profiles.yml

Two-profile sample shipped here:

```yaml
claude-sonnet-cloud:
  model: claude-sonnet-4-6
  ctx: 200000
  reasoning: medium
  allowed: ctx:small,ctx:medium,reasoning:low,reasoning:medium

qwen3-32b-laptop:
  model: qwen3-32b-instruct
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

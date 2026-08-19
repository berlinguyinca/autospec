---
name: autospec-e2e-clone
description: Use when you need to provision an isolated, scaled-down, anonymized clone of a production environment for autospec-test E2E testing (Mode II). Handles snapshot capture, PII anonymization, edge-case data seeding, and URL exposure.
mode: primary
---

# autospec-e2e-clone — Clone Environment Provisioner

Provision an isolated, scaled-down, anonymized clone of a production environment that `autospec-test` can run E2E tests against. The clone exposes a routable URL the autospec-test contract consumes via `e2e.clone_url_env`.

## Self-update mode

Decide this purely from the request text the harness handed you. Do NOT
shell out (no `grep`, `sed`, `[[ =~ ]]`, command substitution, etc.) to
test the user's free-form request — passing it through a shell is what
historically tripped harness permission engines. Read the request, normalize
it in your reasoning (collapse whitespace, trim, lowercase), and if the result is
exactly `update`, this skill enters self-update mode and does NOT run the
normal pipeline.

1. Detect harness by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-e2e-clone/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-e2e-clone.md`
   - Codex CLI:   `~/.codex/prompts/autospec-e2e-clone.md`
2. Re-install from `main` by piping the canonical installer:
   ```
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. Show the diff between the prior installed file(s) and the freshly fetched copy.
4. Stop. Do not enter any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-e2e-clone found; run install.sh first.` and exit.

## Model tier

`reasoning:standard, ctx:120k`

## Required capabilities & harness adapter

This workflow assumes five capabilities. Map each one to your harness's actual tool.

| Capability                  | Claude Code                             | OpenCode                              | Codex CLI                             | Fallback if missing                    |
|-----------------------------|-----------------------------------------|---------------------------------------|---------------------------------------|----------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)         | `task` agent in read-only mode        | shell `grep`                          | Do the search in-thread                |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent                   | spawn nested CLI session              | Do the work in-thread                  |
| Ask the user a question     | `AskUserQuestion`                       | inline prompt                         | inline prompt                         | Ask in response and wait               |
| Subagent model tier         | **Model tier:** `TIER_B` for provisioning work; `TIER_A` for review | top task model + high reasoning | top GPT + reasoning_effort=high | Honor per-phase tier mapping |
<!-- autospec-block:harness-adapter-core -->

## Harness detection

Detect your harness by checking available tools before any phase:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`
   - `TIER_B` = `sonnet`

2. **OpenCode** — a `task` tool with model/tier configuration is available.
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is unavailable (quota, capacity, or auth failure), silently retry with `TIER_A`. Never ask the user.

## Adapter row

| Skill | Trigger | Output |
| --- | --- | --- |
| autospec-e2e-clone | `/autospec-e2e-clone` or autospec-test Mode II gate | `.autospec/clone-url.txt` + provisioned routable URL |

## Contract

Contract file: `.autospec/clone.yml` in the target repository. Load and validate with:

```bash
bash "${AUTOSPEC_SKILLS_DIR:-$HOME/.autospec/skills}/autospec-e2e-clone/scripts/load-contract.sh" <repo_root>
```

Exit codes from `load-contract.sh`:
- `0` — resolved contract JSON on stdout
- `1` — fatal (tool missing, parse error, internal error)
- `2` — refuse-to-run (contract invalid or missing required fields)

Required contract fields:
- `sources` — array of ≥1 source entries (each with `kind`)
- `expose.kind` — one of `docker_compose | k8s_ephemeral_namespace | dedicated_staging_slot | custom_cmd`

Example `.autospec/clone.yml`:

```yaml
sources:
  - kind: postgres
    dsn_env: PROD_DB_URL
    tables_full: [users, products]
    tables_sample: { events: 10000, logs: 5000 }

anonymize:
  rules:
    - table: users
      columns:
        email: hash_with_domain
        ssn: redact

scale_down:
  default_sample: 10000
  foreign_key_aware: true

edge_case_seed:
  consume_from: .autospec/test.yml

expose:
  kind: docker_compose
  compose_file: deploy/docker-compose.clone.yml
  url_template: "http://localhost:8080"
  health_endpoint: /health
  ready_wait_secs: 60
```

## Pipeline

```
1. Snapshot          → capture DB + filesystem + S3 state
2. Anonymize         → redact PII per declarative rules
3. Scale down        → sample large tables (foreign-key-aware)
4. Edge-case seed    → consume edge_case_seeds.require_shapes from .autospec/test.yml
5. Expose URL        → provision routable URL + credentials
6. Health check      → verify URL responds; matrix of expected endpoints
7. Output            → writes .autospec/clone-url.txt for autospec-test gate to read
```

## Components (C1–C10)

| Component | Description | Status |
|-----------|-------------|--------|
| C1 | Skill scaffold + clone.yml contract + JSON Schema | This PR |
| C2 | Snapshot drivers: pg + mysql + sqlite | Pending |
| C3 | Snapshot drivers: s3 + filesystem + custom_cmd | Pending |
| C4 | Anonymize engine + reversible map + pii_assertions | Pending |
| C5 | Scale-down with foreign-key reachability | Pending |
| C6 | Edge-case seed consumer + catalog overlay | Pending |
| C7 | Expose adapter: docker_compose | Pending |
| C8 | Expose adapters: k8s + staging_slot + custom_cmd | Pending |
| C9 | Teardown + autospec-test contract integration | Pending |
| C10 | Synthetic targets + integration tests + dogfood | Done |

## Constraints

- Source DSNs are never logged.
- Snapshot artifacts go to `.autospec/clone-snapshots/` (gitignored).
- PII assertions must pass post-anonymization or provisioning fails (fail-closed).
- Teardown runs automatically on test completion (success or failure).

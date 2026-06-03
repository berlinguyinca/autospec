# autospec-doc

Harness-neutral, autospec-family **per-audience documentation generator**.
Generates and maintains project documentation for four audiences — **user**,
**developer**, **admin**, and **general** — as a verifiable, docs-as-tests
artifact: every code example is executed against the repo, the visual palette
is single-sourced, and the whole corpus is concatenated into an `llms-full.txt`
for downstream LLM consumption.

Incremental by default (cheap, scoped to what changed), with `--full`,
`--audit`, `--audience <name>`, and an `init` scaffolder. Every operation routes
through `scripts/doc-orchestrator.mjs`.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

> **Status:** scaffold (issue #916). This issue ships the lock-step trio with
> the required structural sections, packaging, and the `doc-orchestrator.mjs`
> router stub. The generators (`gen-audience-docs.mjs`, `verify-examples.mjs`,
> `doc-style.mjs`, `gen-llms-full.mjs`), the config schema + folder contract,
> and the Phase 4 / Phase 5.5 / sweep / define wiring land in follow-up child
> issues #917-#924.

## Subcommands

| Form | Behavior |
| --- | --- |
| `/autospec-doc` | Incremental — regenerate scopes affected since last generation. The cheap default. |
| `/autospec-doc --full` | Regenerate everything + run the completeness audit. |
| `/autospec-doc --audit` | Read-only completeness/drift report; writes nothing. |
| `/autospec-doc --audience <name>` | Regenerate one audience only. |
| `/autospec-doc init` | Scaffold the `documentation:` config + starter doc scopes. |

Every non-`init` subcommand exits `2` when no `documentation:` config exists in
`.autospec/autospec.yml`: run `/autospec-doc init` first to scaffold it.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-doc/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-doc/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-doc/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-doc/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-doc
./install.sh --harness all
```

## Invocation

```
/autospec-doc [--full | --audit | --audience <name> | init]
```

## Required structural sections

Every harness body (`SKILL.md` / `codex/prompt.md` / `opencode/agent.md`)
carries these sections, so the decomposer and the repo validator recognise it as
a conformant autospec skill:

1. **Startup self-update** — canonical self-update block, `SKILL_NAME=autospec-doc`.
2. **Self-update mode** — pure-prose dispatch on `^\s*update\s*$`.
3. **Model tier** — AGENTS.md Tier A/B + harness-detection + adapter row.
4. **Subcommand contract** — the five-form `/autospec-doc` routing table.

## Config

`.autospec/autospec.yml` `documentation:` block. `init` seeds the four default
audiences (user → `docs/user`, developer → `docs/developer`, admin →
`docs/admin`, general → `docs/general`) only when `audiences:` is empty.
*(Schema + folder-contract path constants land in #917.)*

## Self-update

```
/autospec-doc update
```

Or re-run the installer with `--update`:

```bash
./install.sh --harness all --update
```

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-define`](../autospec-define/README.md) | Plans features; its auto-docs step redirects here (#924). |
| [`autospec-run`](../autospec-run/README.md) | Phase 4 gains a `regenerate` self-heal action calling this skill (#922). |
| [`autospec-sweep`](../autospec-sweep/README.md) | Its docs-drift check redirects to `/autospec-doc --full` (#924). |

## License

MIT — see the repository [LICENSE](../../LICENSE) file.

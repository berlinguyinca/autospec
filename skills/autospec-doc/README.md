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
> router stub. The config schema + folder contract (`scripts/doc-config.mjs`)
> and the working `init` scaffolder landed in #917. The generators
> (`gen-audience-docs.mjs`, `verify-examples.mjs`, `doc-style.mjs`,
> `gen-llms-full.mjs`) and the Phase 4 / Phase 5.5 / sweep / define wiring land
> in follow-up child issues #918-#924.

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

`.autospec/autospec.yml` `documentation:` block, parsed by
`scripts/doc-config.mjs`. `init` seeds the four default audiences (user →
`docs/user`, developer → `docs/developer`, admin → `docs/admin`, general →
`docs/general`) only when `audiences:` is empty.

```yaml
documentation:
  audiences:
    - {name: user,      path: docs/user,      focus: "tasks, workflows, how to use features"}
    - {name: developer, path: docs/developer, focus: "architecture, APIs, extending"}
    - {name: admin,     path: docs/admin,     focus: "install, configure, operate, troubleshoot"}
    - {name: general,   path: docs/general,   focus: "what it is, why it matters, plain language"}
  style:
    palette: light-blue        # named preset (default: light-blue)
  examples:
    verify: true               # docs-as-tests on/off (default: true)
    sandbox: worktree          # example execution isolation (default: worktree)
  # documentation.auto_regenerate — opt-in to automatic doc regeneration in runs.
  # Default: false. Three ways to enable for a given run (highest precedence first):
  #   1. Add "docs: skip" to the issue body to suppress regardless of other settings.
  #   2. Add "docs: generate" to the issue body to enable for that issue only.
  #   3. Set auto_regenerate: true here, or pass --with-docs (AUTOSPEC_WITH_DOCS=1).
  documentation:
    auto_regenerate: false
```

**Folder contract** per audience: `index.md`, `getting-started.md`,
`tutorials/<feature>.md`, `features/<feature>.md`; **developer** additionally
gets `architecture/` + `api/`; **admin** links `docs/runbooks/`; shared assets
live under `docs/assets/{screenshots,diagrams,transcripts}/`. The path constants
are exported from `scripts/doc-config.mjs` as `FOLDER_CONTRACT`.

### Migration note

The `style`, `examples`, and `documentation` keys are **new and optional** —
existing `documentation:` configs keep working unchanged. If your repo already
declares `audiences:` (including custom audiences such as `operators` or
`security-reviewers`, or legacy entries keyed by `id`/`label`), those entries
are preserved **verbatim**: the loader never rewrites their
`name`/`path`/`focus`/`require_scope` fields and does **not** inject the four
defaults when any audience is already declared. To adopt the new behavior, add a
`style:` and/or `examples:` block; omit them to accept the defaults
(`palette: light-blue`, `examples.verify: true`, `examples.sandbox: worktree`).

The `documentation.auto_regenerate` key (added in #953) defaults to `false` —
**doc regeneration is off by default**. To enable it globally set
`documentation.auto_regenerate: true` in your config, or pass `--with-docs`
(`AUTOSPEC_WITH_DOCS=1`) at run time. To enable it for a single issue only, add
a `docs: generate` line to the issue body. To suppress it regardless of any
setting, add `docs: skip` to the issue body. No action is required to upgrade.

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


# autospec-doc workflow (harness-neutral)

Generate and maintain **per-audience project documentation** as a first-class,
verifiable artifact. The product is documentation infrastructure: every page is
audience-targeted (user / developer / admin / general), every code example is
executed (docs-as-tests), the visual palette is single-sourced, and the whole
corpus is concatenated into an `llms-full.txt` for downstream LLM consumption.

Goal: documentation that a non-author can actually follow, that never silently
drifts from the code, whose examples are proven to run, and whose token cost
stays bounded through incremental regeneration. This skill is the operator
entry point; the heavy lifting lives in `doc-orchestrator.mjs` and the
per-capability generators it dispatches.

This SKILL.md is the **scaffold contract** (issue #916). The subcommand router
exists as a stub; the generator scripts (`gen-audience-docs.mjs`,
`verify-examples.mjs`, `doc-style.mjs`, `gen-llms-full.mjs`) are header+stub
here and filled in by downstream issues #917-#921. Sections below marked
*(filled in by #NNN)* are intentional stubs.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-doc -->

## Self-update mode

If the argument matches the regex `^\s*update\s*$` (case-insensitive,
whitespace-padded), this skill enters self-update mode and does not run the
normal pipeline. This section is pure prose: never interpolate or shell out the
operator's argument text.

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-doc/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-doc.md`
   - Codex CLI:   `~/.codex/prompts/autospec-doc.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not enter the documentation pipeline. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-doc found; run install.sh first.` and exit.

## Model tier

Tier A/B model resolution and the harness-detection protocol are inherited
verbatim from `AGENTS.md` (`## Subagent model selection (two-tier, cost-aware)`
and `## Subagent vs inline decision matrix`). Audience-content authoring and the
completeness audit run at Tier A (spec work — fresh-eyes reasoning produces
better prose and catches gaps); deterministic generation, example verification,
and palette resolution run at Tier B (implementation work).

> **Model tier:** TIER_A (spec work) for the per-audience content pass and the completeness audit; TIER_B (implementation work) for deterministic generation, example verification, and style — resolved at startup per the harness detection below.

### Required capabilities & harness adapter

This workflow assumes the following capabilities. Map each to your harness's
actual tool; if a capability is missing, use the listed fallback.

| Capability                  | Claude Code                             | OpenCode                              | Codex CLI                                | Fallback if missing                              |
|-----------------------------|-----------------------------------------|---------------------------------------|------------------------------------------|--------------------------------------------------|
| Audience content authoring  | `Agent` (subagent_type=general-purpose) | `task` agent, await output            | run the authoring pass inline            | Author in-thread (more context cost)             |
| Completeness / gap audit    | a **separate** `Agent` dispatch         | a **separate** `task` agent           | a separate inline judging pass           | Audit in-thread, never reuse the authoring pass  |
| Read-only repo research     | `Agent` (subagent_type=Explore)         | `task` agent in read-only mode        | shell `grep`/`rg` read-only              | Search in-thread with `rg`/`grep`                |
| Ask the user a question     | `AskUserQuestion`                       | inline prompt                         | inline prompt                            | Ask in the response and wait for the next turn   |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
| Subagent dispatch policy    | per AGENTS.md decision matrix           | per AGENTS.md decision matrix         | per AGENTS.md decision matrix            | inline with main-session token cost              |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in
the repo root — recognized by Claude Code, OpenCode, and Codex.

## Harness detection (run once at skill start, before generation begins)

Detect your harness by checking available tools before any dispatch:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)
2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning
3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; passes run inline.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness, silently retry
the same subagent dispatch with `TIER_A`. Preserve parent context on retry.
Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run.

## Subcommand contract

`/autospec-doc` routes every operation through `doc-orchestrator.mjs`.
There are five forms; resolve the active repo's `<owner/name>` once and reuse it.

```
/autospec-doc                     # incremental — regenerate scopes affected since last generation
/autospec-doc --full              # regenerate everything + run the completeness audit
/autospec-doc --audit             # read-only completeness/drift report; writes nothing
/autospec-doc --audience <name>   # regenerate one audience only (user|developer|admin|general|custom)
/autospec-doc init                # scaffold the documentation: config + starter doc scopes
```

- **(bare) incremental** — the default and the cheap path. Scopes only the docs
  affected since the last generation (changed features, drifted examples). Keeps
  per-run token cost bounded; never regenerates the whole corpus. Requires a
  `documentation:` config. *(scope-diff + generation filled in by #918/#919.)*
- **`--full`** — regenerate every audience's pages from scratch and then run the
  completeness audit. The expensive, exhaustive path. Requires config.
  *(filled in by #918/#923.)*
- **`--audit`** — read-only. Reports missing scopes, stale verified examples,
  and audience coverage gaps without writing any file. Requires config.
  *(filled in by #919/#923.)*
- **`--audience <name>`** — regenerate exactly one audience's docs. Requires
  config; `<name>` must resolve to a configured (or default) audience.
  *(filled in by #918.)*
- **`init`** — the bootstrap. Scans the repo (features, entry points, existing
  `docs/`) and scaffolds a `documentation:` block in `.autospec/autospec.yml`
  plus starter doc scopes under `docs/<audience>/` per the folder contract. This
  is the ONLY subcommand that runs WITHOUT an existing `documentation:` config —
  it is what creates that config. *(scaffolding logic filled in by #917.)*

**Config gate.** Every non-`init` subcommand exits `2` when no `documentation:`
config is present in `.autospec/autospec.yml`: there is nothing to generate
until `init` has scaffolded the config. The orchestrator enforces this gate
deterministically; `init` is exempt by design.

## Audience & content model

Four default audiences, seeded by `init` only when `documentation.audiences` is
empty: **user** (`docs/user`, "tasks, workflows, how to use features"),
**developer** (`docs/developer`, "architecture, APIs, extending"), **admin**
(`docs/admin`, "install, configure, operate, troubleshoot"), and **general**
(`docs/general`, "what it is, why it matters, plain language"). Operators may
declare custom audiences. *(config schema + folder-contract constants filled in
by #917; per-audience generators by #918.)*

Folder contract: `docs/<audience>/{index.md,getting-started.md,tutorials/<feature>.md,features/<feature>.md}`;
developer adds `architecture/` + `api/`; admin links `docs/runbooks/`; shared
assets live under `docs/assets/{screenshots,diagrams,transcripts}/`.

## Docs-as-tests

Every fenced code example in generated docs is executed against the repo, so
documentation cannot silently drift from working code. Verified examples carry a
`<!-- example-verified: <head-sha> <ISO-date> -->` marker; a re-run on a newer
HEAD that no longer matches re-verifies or flags the example as stale.
The verification engine is `verify-examples.mjs`: it executes every
tagged example in a fresh worktree off origin/main (network-restricted, 60s
per-example timeout), embeds captured output in an adjacent ` ```output ` block,
and a failing example fails generation. `check-doc-drift.sh` reports an
`example_stale` entry when a marker SHA predates the newest commit touching the
scope's `src_globs` (same self-heal path as `visual_stale`).

## Style & palette

The visual palette is **single-sourced** in `doc-style.mjs` — no other
file may hardcode the palette hexes, and `validate.sh` enforces this.
The palette also themes generated mermaid diagrams. *(palette resolution +
mermaid theming filled in by #920.)*

## llms-full.txt

`--full` (and the orchestrator's concatenation step) emit an `llms-full.txt` at
the repo root: the whole verified corpus, delimited per audience/feature with
`<!-- llms: audience=<a> feature=<f> -->` markers, plus a `.llm-manifest.json`
index. *(concatenator + manifest fill filled in by #921.)*

## Pipeline wiring (downstream)

Phase 4 of `/autospec-run` gains a `regenerate` self-heal action that calls
`/autospec-doc` when docs drift is detected; Phase 5.5 gains a
docs-completeness dimension; `/autospec-sweep --full` and `/autospec-define`'s
auto-docs step redirect to `/autospec-doc`. *(wiring filled in by
#922/#923/#924.)*

## Worktree assert (regenerate commit)

The regenerate commit step runs inside the implementer's PR-branch worktree
(already a linked worktree off `origin/main`, never the primary checkout).
Before committing any regenerated content, the doc orchestrator MUST call
`worktree-guard.sh assert` and verify it exits 0.

<!-- doc-regen-assert:begin -->
```bash
# MANDATORY assert gate before the doc regenerate commit.
# MUST exit 0 before any commit. A non-zero exit (in_primary_checkout / dirty /
# stale_base) is NEVER worked around — stop and surface the code_health identifier.
if ! bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/worktree-guard.sh assert; then
  echo "worktree-guard assert failed before doc regenerate commit; aborting" >&2
  exit 1
fi
```
<!-- doc-regen-assert:end -->

## Invocation

```
/autospec-doc [--full | --audit | --audience <name> | init]
```

> **Model tier:** TIER_A (spec work) for the audience-content authoring pass and the completeness audit; TIER_B (implementation work) for deterministic generation, example verification, and style — resolved at startup per the harness detection above.

The skill resolves the harness and tiers, runs the startup self-update preflight,
then dispatches the parsed subcommand to `doc-orchestrator.mjs`. On a
non-`init` subcommand with no `documentation:` config, it surfaces the orchestrator's
exit-`2` message instructing the operator to run `/autospec-doc init` first.

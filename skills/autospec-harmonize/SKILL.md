---
name: autospec-harmonize
description: Use when the operator wants to harmonize a codebase's design tokens — discover palette/type/spacing drift, generalize a baseline design, generate variants (minimal, high-contrast, dense, bold), preview them, and produce a dated migration spec. Triggers on "harmonize design", "design tokens", "style drift", "palette inconsistency", "design migration spec".
---

# autospec-harmonize workflow (harness-neutral)

Orchestrate a 6-stage design-token harmonization pipeline for a target
codebase: discover → generalize → variants → preview → pick → gen-migration-spec.
The result is a dated design-migration spec ready for `/autospec-define`.

Goal: a harness-neutral, autospec-family way to identify design-token drift in
a codebase, surface a shortlist of harmonized variants, let the operator pick
one, and emit a structured migration spec — all on small-context local models.

Manage your own context — never exceed 60%. Delegate per-stage work to
subagents whenever your harness supports it; do not carry raw stage transcripts
in the orchestrator context.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-harmonize -->

## Self-update mode

If the argument matches the regex `^\s*update\s*$` (case-insensitive,
whitespace-padded), this skill enters self-update mode and does not run the
normal pipeline. This section is pure prose: never interpolate or shell out the
operator's argument text.

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-harmonize/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-harmonize.md`
   - Codex CLI:   `~/.codex/prompts/autospec-harmonize.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not enter the harmonize pipeline. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-harmonize found; run install.sh first.` and exit.

## Model tier

Tier A/B model resolution and the harness-detection protocol are inherited
verbatim from `AGENTS.md` (`## Subagent model selection (two-tier, cost-aware)`
and `## Subagent vs inline decision matrix`). Design-pick reasoning runs at
Tier A (spec work); per-stage pipeline work runs at Tier B (implementation
work), escalating to Tier A for the generalize and migration-spec stages.

> **Model tier:** The pick gate and migration-spec generation dispatch at Tier A (spec work);
> discovery, generalize, variants, and preview dispatch at TIER_B (implementation work),
> resolved at startup per the harness detection below.

### Required capabilities & harness adapter

This workflow assumes the following capabilities. Map each to your harness's
actual tool; if a capability is missing, use the listed fallback.

| Capability                  | Claude Code                             | OpenCode                              | Codex CLI                                | Fallback if missing                              |
|-----------------------------|-----------------------------------------|---------------------------------------|------------------------------------------|--------------------------------------------------|
| Per-stage worker            | `Agent` (subagent_type=general-purpose) | `task` agent, await output            | run the stage inline (no subagents)      | Run the stage in-thread (more context cost)      |
| Ask the user a question     | `AskUserQuestion`                       | inline prompt                         | inline prompt                            | Ask in the response and wait for the next turn   |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
| Subagent dispatch policy    | per AGENTS.md decision matrix           | per AGENTS.md decision matrix         | per AGENTS.md decision matrix            | inline with main-session token cost              |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in
the repo root — recognized by Claude Code, OpenCode, and Codex.

## Harness detection (run once at skill start)

Detect your harness by checking available tools before any dispatch:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)
2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning
3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; stages run inline.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness, silently retry
the same subagent dispatch with `TIER_A`. Preserve parent context on retry.
Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run.

## Pipeline overview

The harmonize pipeline sequences these 6 stages:

1. **Discover** — extract design tokens from the target codebase (source CSS/SCSS
   or live URL runtime extraction). Writes `<out>/discovered-tokens.json` +
   `<out>/inventory.md`.
2. **Generalize** — collapse the token profile into one `id:"baseline"` variant
   conforming to the variant schema.
3. **Variants** — expand the baseline into an array of design variants for the
   requested axes (minimal, high-contrast, dense, bold, vendor-blend). The
   baseline is always index 0; the result array has ≥3 elements including baseline.
4. **Preview** — render a self-contained HTML gallery from the variants array.
   Writes `<out>/preview/index.html`. Uses Playwright screenshots when available;
   falls back to color-swatch HTML when `AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1` or
   Playwright is absent.
5. **Pick gate** — present the shortlisted variants to the operator via
   `AskUserQuestion` and record their choice (see **Pick gate** section below).
   `--no-live-preview` suppresses the live-preview offer. No winner → exits 0
   without writing a migration spec.
6. **Gen migration spec** — render a dated `docs/specs/<date>-harmonize-<slug>-design.md`
   from the chosen variant + inventory. Print the `/autospec-define <path>`
   handoff command so the operator can kick off implementation.

The orchestrator script (`harmonize.sh`) is **non-interactive**: the pick gate
lives in this SKILL.md and is only active when running under a harness. When
`harmonize.sh` is invoked directly (e.g. from a test), it uses the default pick
(baseline or first non-baseline variant) for the migration spec without
prompting.

## Invocation

```
/autospec-harmonize [--root <dir>] [--url <u>] [--source-only]
                    [--pages <list>] [--variants <axes>] [--num-variants <n>]
                    [--no-live-preview] [--out <dir>]
```

- `--root <dir>` — the codebase root to analyze (defaults to current working directory).
- `--url <u>` — extract tokens from a live URL via runtime extractor (falls back to source if unavailable).
- `--source-only` — skip runtime extraction; always use source CSS/SCSS.
- `--pages <list>` — comma-separated list of URL paths to sample during runtime extraction.
- `--variants <axes>` — comma-separated variant axes to generate beyond baseline (default: `minimal,high-contrast`).
- `--num-variants <n>` — total number of variants to generate (including baseline; default: 3).
- `--no-live-preview` — suppress the live-preview offer in the pick gate.
- `--out <dir>` — output directory for all artifacts (default: `.autospec/harmonize/<slug>`).

## Stage scripts

Each stage is a sibling script that the orchestrator shells out to. The
orchestrator resolves them from `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}`,
falling back to the script's own directory:

| Stage | Script |
|-------|--------|
| Source extraction | `extract-source.mjs` |
| Runtime extraction | `extract-runtime.mjs` |
| Discover orchestration | `design-discover.sh` |
| Generalize | `design-generalize.mjs` |
| Variants | `design-variants.mjs` |
| Preview | `design-preview.mjs` |
| Migration spec | `gen-migration-spec.mjs` |

## Pick gate

After the preview stage, use `AskUserQuestion` to present the shortlist of
variants to the operator:

1. Show each variant's `id`, `axis`, and a one-line `design_md` summary.
2. Include the path to `<out>/preview/index.html` so the operator can open it.
3. If `--no-live-preview` is NOT set, offer to open the preview in a browser.
4. Ask the operator to type the `id` of their preferred variant, or `none` to
   exit without generating a migration spec.
5. If the operator types `none` (or an empty response), print
   `No variant selected — exiting without a migration spec.` and exit 0.
6. Validate that the chosen `id` exists in the variants array. If invalid, ask
   again (up to 3 attempts); on the third failure, exit 0 with an error message.
7. Proceed to Stage 6 with the chosen variant `id`.

When running `harmonize.sh` directly (non-interactive, no harness), skip this
gate entirely and use the first non-baseline variant (or baseline if only one
exists) as the default pick.

## Stage failure handling

Each stage failure emits its `code_health` line to stderr and continues where
possible:

- If **discovery** yields an empty token profile (no palette, type_scale, or
  spacing entries), abort with exit code 1 and print
  `code_health:harmonize_discover_empty — no design tokens found in <root>`.
- If **generalize** fails, emit `code_health:harmonize_generalize_failed` and
  abort.
- If **variants** fails, emit `code_health:harmonize_variants_failed` and abort.
- If **preview** fails (non-fatal), emit `code_health:harmonize_preview_failed`
  and continue — the pick gate can still show the text summary.
- If **gen-migration-spec** fails, emit `code_health:harmonize_spec_failed` and
  report the error to the operator without aborting the skill run.

## Output artifacts

All artifacts land under `--out <dir>`:

| Path | Stage | Description |
|------|-------|-------------|
| `discovered-tokens.json` | Discover | Token profile (schema-validated) |
| `inventory.md` | Discover | Human-readable inventory + inconsistency report |
| `baseline.json` | Generalize | Baseline variant JSON |
| `variants.json` | Variants | Full variants array JSON |
| `preview/index.html` | Preview | Self-contained HTML gallery |
| `docs/specs/<date>-harmonize-<slug>-design.md` | Gen spec | Dated migration spec |

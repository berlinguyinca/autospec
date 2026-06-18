# autospec-harmonize

Harness-neutral, autospec-family **design-token harmonization pipeline**.
Discovers palette/typography/spacing drift in a codebase, generalizes a
baseline design variant, generates a shortlist of harmonized variants
(minimal, high-contrast, dense, bold, vendor-blend), renders an HTML preview
gallery, records the operator's variant pick, and emits a dated migration spec
ready for `/autospec-define`.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Pipeline stages

| # | Stage | Script | Output |
|---|-------|--------|--------|
| 1 | Discover | `design-discover.sh` | `discovered-tokens.json`, `inventory.md` |
| 2 | Generalize | `design-generalize.mjs` | `baseline.json` |
| 3 | Variants | `design-variants.mjs` | `variants.json` (≥3 elements incl. baseline) |
| 4 | Preview | `design-preview.mjs` | `preview/index.html` |
| 5 | Pick gate | SKILL.md (`AskUserQuestion`) | operator's chosen variant id |
| 6 | Gen migration spec | `gen-migration-spec.mjs` | `docs/specs/<date>-harmonize-<slug>-design.md` |

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-harmonize/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-harmonize/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-harmonize/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-harmonize/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-harmonize
./install.sh --harness all
```

## Invocation

```
/autospec-harmonize [--root <dir>] [--url <u>] [--source-only]
                    [--pages <list>] [--variants <axes>] [--num-variants <n>]
                    [--no-live-preview] [--out <dir>]
```

- `--root <dir>` — codebase root to analyze (defaults to cwd).
- `--url <u>` — extract tokens from a live URL via runtime extractor.
- `--source-only` — skip runtime extraction; always use source CSS/SCSS.
- `--pages <list>` — comma-separated URL paths to sample during runtime extraction.
- `--variants <axes>` — comma-separated variant axes beyond baseline (default: `minimal,high-contrast`).
- `--num-variants <n>` — total variants including baseline (default: 3).
- `--no-live-preview` — suppress the live-preview offer in the pick gate.
- `--out <dir>` — output directory for all artifacts.

## Orchestrator script

`scripts/harmonize.sh` is the non-interactive stage sequencer. It is called by
the SKILL.md workflow; it can also be used directly in CI or tests:

```bash
AUTOSPEC_HARMONIZE_LLM_STUB=1 \
AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
bash skills/autospec-harmonize/scripts/harmonize.sh \
  --root path/to/app \
  --source-only \
  --no-live-preview \
  --out /tmp/harmonize-out
```

Set `AUTOSPEC_SCRIPTS_DIR` to point at a custom scripts directory (otherwise
defaults to the script's own sibling directory).

## Required structural sections

Every harness body (`SKILL.md` / `codex/prompt.md` / `opencode/agent.md`)
carries these 7 sections in order, so the decomposer and repo validator
recognise it as a conformant autospec skill:

1. **Startup self-update** — canonical self-update block, `SKILL_NAME=autospec-harmonize`.
2. **Self-update mode** — pure-prose dispatch on `^\s*update\s*$`.
3. **Model tier** — AGENTS.md Tier A/B + harness-detection + adapter row.
4. **Harness detection** — per-harness TIER_A/TIER_B resolution.
5. **Pipeline overview** — the 6 stages.
6. **Pick gate** — `AskUserQuestion` variant selection.
7. **Stage failure handling** — per-stage `code_health` signals.

## Self-update

```
/autospec-harmonize update
```

Or re-run the installer with `--update`:

```bash
./install.sh --harness all --update
```

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-define`](../autospec-define/README.md) | Receives the `/autospec-define <spec-path>` handoff from Stage 6. |
| [`autospec-design`](../autospec-design/README.md) | Complementary design workflow. |
| [`autospec`](../autospec/README.md) | Full plan-and-ship pipeline. |

## License

MIT — see the repository [LICENSE](../../LICENSE) file.

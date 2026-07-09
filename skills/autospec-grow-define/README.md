# autospec-grow-define

Planning half of the **autospec-growth** suite. Opt-in per repo via
`.autospec/growth.yml` (mirrors `autospec-fab`'s `.autospec/fab.yml` gate) — a
repo without this file is inert to `/autospec-grow-define`.

Runs the planning half of one growth cycle: measure current metrics → fan out
the 6-lens growth researcher roster → adversarially verify + ROI/severity-rank
candidates through a deterministic pipeline → decompose survivors into linked
GitHub issues (`growth:artifact` / `growth:outbound`). Stops after
decomposition and hands off to `/autospec-grow-run` to drain the backlog.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-grow-define/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
# Claude Code only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-grow-define/install.sh | sh -s -- --harness claude
# OpenCode only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-grow-define/install.sh | sh -s -- --harness opencode
# Codex CLI only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-grow-define/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-grow-define
./install.sh --harness all
```

## Configure `.autospec/growth.yml`

The skill is inert until this file exists and validates. See the design spec
at `docs/specs/2026-07-08-autospec-growth-design.md` for the full schema
(`product`, `site`, `channels`, `targets`, `measurement`, `approval`,
`guardrails`). Every credential is referenced by env-var name only
(`token_env`) — an inline secret fails validation.

## What it does

- **Configuration gate** — reads and validates `.autospec/growth.yml` via
  `validate-growth-config.sh`; exits with a one-line pointer if absent or
  invalid.
- **Phase G0 (Measure, optional)** — normalizes any config-pointed or fixture
  metric payloads via `growth-measure.sh --normalize`; falls back to an empty
  envelope. Live provider API calls land in a later plan.
- **Phase G1 (Lens research)** — 6 parallel Tier-A subagents, one per lens
  (`technical-seo`, `keyword-gap`, `content-opportunity`, `community`,
  `directory`, `backlink`), each budget-scaled by
  `growth-source-weights.sh`. Each returns candidates matching
  `skills/autospec-shared/schemas/growth-candidate.schema.json`.
- **Phase G2 (Pipeline)** — a Tier-A adversarial-verify subagent per candidate
  (default `real:false` on doubt), then the deterministic
  `grow-define-pipeline.sh` validates, dedups against the ledger, ranks, and
  slices to the configured top-N.
- **Phase G3 (Decompose)** — routes simple/outbound candidates straight to
  `grow-define-file-issues.sh`; routes multi-issue-worthy artifacts through
  `autospec-define`'s Phase 3 decomposition, then labels the result.

When Phase G3 completes the skill stops and prints a summary pointing at
`/autospec-grow-run`.

## Subagent model selection

Lens research (G1), adversarial verify (G2), and decomposition (G3, delegated
to `autospec-define` Phase 3) all run on **Tier A** per `AGENTS.md`: top model
with extended thinking. Config validation and the deterministic pipeline
scripts (`growth-measure.sh`, `grow-define-pipeline.sh`,
`grow-define-file-issues.sh`) are mechanical — Tier B or no subagent at all.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-grow-run`](../autospec-grow-run/README.md) | Drains the growth backlog — wraps `/autospec-run` for `growth:artifact` issues, routes `growth:outbound` issues through the approval pipeline. |
| [`autospec-define`](../autospec-define/README.md) | Supplies the Phase 3 decomposition path for multi-issue-worthy growth artifacts. |
| [`autospec-fab`](../autospec-fab/README.md) | The sibling opt-in-per-repo pattern (`.autospec/fab.yml`) this skill's config gate mirrors. |

## Self-update

Once installed, run the skill with the literal argument `update` to refresh in
place. The skill detects its harness, re-runs the canonical install one-liner
with `--update`, shows the diff, and stops without entering Phase G0:

```
/autospec-grow-define update
```

Or re-run the per-skill installer with `--update`:

```bash
./install.sh --harness all --update
```

## Usage

```
/autospec-grow-define
```

(Replace `/` with `@` for OpenCode, or invoke through Codex CLI as a slash
command. Requires a validated `.autospec/growth.yml` in the current repo.)

## License

MIT — see the repository [LICENSE](../../LICENSE) file.

# autospec-grow-run

Implementation half of the **autospec-growth** suite. Opt-in per repo via
`.autospec/growth.yml` (same gate `autospec-grow-define` uses) — a repo
without this file is inert to `/autospec-grow-run`.

Picks up where `/autospec-grow-define` stops: drains `growth:artifact`
issues via `/autospec-run` behind a content-quality gate, drafts and gates
`growth:outbound` issues through an ethics + cadence + relevance pipeline
into a human approval queue (package-only, never auto-posts), handles
approval-issue state, and measures/attributes shipped work via live
adapters that re-weight the next `/autospec-grow-define` cycle.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-grow-run/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
# Claude Code only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-grow-run/install.sh | sh -s -- --harness claude
# OpenCode only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-grow-run/install.sh | sh -s -- --harness opencode
# Codex CLI only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-grow-run/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-grow-run
./install.sh --harness all
```

## Configure `.autospec/growth.yml`

The skill is inert until this file exists and validates. See the design spec
at `docs/specs/2026-07-08-autospec-growth-design.md` for the full schema
(`product`, `site`, `channels`, `targets`, `measurement`, `approval`,
`guardrails`). Every credential is referenced by env-var name only
(`token_env`) — an inline secret fails validation.

## What it does

- **Configuration gate (R0)** — reads and validates `.autospec/growth.yml`
  via `validate-growth-config.sh`; exits with a one-line pointer if absent or
  invalid.
- **R1 — Artifact drain** — invokes `/autospec-run` per open
  `growth:artifact` issue, then runs a content-quality gate (deterministic
  keyword-density/citation precheck + a Tier-A reviewer, 5-attempt adaptive
  retry) before that issue is allowed to auto-merge, additive to the
  standard reviewer/ethics/secaudit gates.
- **R2 — Outbound draft → gate → queue** — drafts outbound copy per open
  `growth:outbound` issue, structurally validates it, runs the ethics gate
  (disclosure + LLM review, 5-attempt retry) and a cadence/relevance check,
  then files a `growth/needs-approval` control issue carrying the built
  body. Package-only — never posts on its own.
- **R3 — Handle approvals** — routes open `growth/needs-approval` control
  issues by label (`approved` / `edited` / `rejected`); `approved` only ever
  produces a one-click-publish comment, never an actual post.
- **R4 — Measure & attribute** — runs the configured measurement adapters
  (GitHub, analytics, GSC, rank), degrading any provider to `{}` on missing
  credentials or fetch failure; attributes ledger outcomes against the
  before/after envelopes and recomputes source weights to bias the next
  `/autospec-grow-define` cycle.

## Subagent model selection

Content-quality review (R1), outbound drafting and the ethics gate (R2/R3),
and attribution synthesis (R4) all run on **Tier A** per `AGENTS.md`: top
model with extended thinking. Config validation and the deterministic
pipeline scripts (`validate-outbound-draft.sh`,
`growth-content-quality-precheck.sh`, `growth-outbound-queue.sh`,
`growth-adapter-*.sh`, `growth-attribute.sh`) are mechanical — Tier B or no
subagent at all.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-grow-define`](../autospec-grow-define/README.md) | Planning half of the suite — lens research, adversarial verify, and decomposition into `growth:artifact`/`growth:outbound` issues. |
| [`autospec-run`](../autospec-run/README.md) | Supplies the implementation/merge machinery this skill wraps per `growth:artifact` issue. |
| [`autospec-fab`](../autospec-fab/README.md) | The sibling opt-in-per-repo pattern (`.autospec/fab.yml`) this skill's config gate mirrors. |

## Self-update

Once installed, run the skill with the literal argument `update` to refresh in
place. The skill detects its harness, re-runs the canonical install one-liner
with `--update`, shows the diff, and stops without entering Phase R0:

```
/autospec-grow-run update
```

Or re-run the per-skill installer with `--update`:

```bash
./install.sh --harness all --update
```

## Usage

```
/autospec-grow-run
```

(Replace `/` with `@` for OpenCode, or invoke through Codex CLI as a slash
command. Requires a validated `.autospec/growth.yml` in the current repo,
typically after `/autospec-grow-define` has filed issues.)

## License

MIT — see the repository [LICENSE](../../LICENSE) file.

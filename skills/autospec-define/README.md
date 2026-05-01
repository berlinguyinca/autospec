# autospec-define

Planning half of the **autospec** suite. Runs Phases 0–3 of the autospec pipeline
(bootstrap repo if missing → investigate → design → spec → decomposed GitHub issues),
then stops with a handoff message pointing the user at `/autospec-run --profile <name>`
to begin autonomous implementation.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-define/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
# Claude Code only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-define/install.sh | sh -s -- --harness claude
# OpenCode only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-define/install.sh | sh -s -- --harness opencode
# Codex CLI only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-define/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-define
./install.sh --harness all
```

## What it does

- **Phase 0** — Bootstrap a fresh GitHub repo if the cwd has no git/remote (asks
  name / visibility / owner once).
- **Phase 1** — Read-only research subagent maps the codebase.
- **Phase 2** — Structured 5-section brainstorm (architecture / API / data / errors /
  testing), written to `docs/specs/YYYY-MM-DD-<topic>-design.md`.
- **Phase 3** — Foreground subagent decomposes the spec into an EPIC umbrella plus
  N self-contained children pre-staged for 32B-class local LLMs.

When Phase 3 completes the skill stops and prints:

```
Phase 3 complete. Run /autospec-run --profile <name> to begin implementation.
```

## Subagent model selection (Tier A only)

This skill is the planning half — every subagent it dispatches (Phase 1 research, Phase 3 decomposition, Phase 3.5 review-and-label) runs on **Tier A** per `AGENTS.md`: top model with extended/maximum thinking. Spec quality is the bottleneck, so we pay for the top model here once rather than N times in cheap-implementer corrections downstream.

Phase 2 has no subagent dispatch — invoke this skill with your top-tier model so the orchestrator itself writes the spec at full quality.

The Tier B (cheaper + medium thinking) tier is **not used** by this skill; it's reserved for `autospec-run`'s implementation loop.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec`](../autospec/README.md) | The full pipeline (Phases 0–6) when you want one skill that plans **and** ships. |
| [`autospec-run`](../autospec-run/README.md) | Implementation half (Phases 4–6). Picks up where `autospec-define` stops; supports `--profile <name>` filtering against `~/.autospec/model-profiles.yml`. |
| [`autospec-classify`](../autospec-classify/README.md) | Standalone retro-labeler for already-existing `auto-implement` issues; applies the Phase 3.5 `ctx:*` / `reasoning:*` rubric. |

## Self-update

Once installed, run the skill with the literal argument `update` to refresh in
place. The skill detects its harness, re-runs the canonical install one-liner
with `--update`, shows the diff, and stops without entering Phase 0:

```
/autospec-define update
```

Or re-run the per-skill installer with `--update`:

```bash
./install.sh --harness all --update
```

## Usage

```
/autospec-define Build a Slack bot that posts a daily standup digest
```

(Replace `/` with `@` for OpenCode, or invoke through Codex CLI as a slash command.)

## License

MIT — see the repository [LICENSE](../../LICENSE) file.

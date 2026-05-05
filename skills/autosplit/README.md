# autosplit

Existing-spec split shortcut for the **autospec** suite. It splits an already-written, tracked `docs/specs/*.md` into the same
EPIC plus `auto-implement` child issue queue. Invoke with `split existing spec`,
`split latest spec`, or an explicit path such as
`turn docs/specs/2026-05-01-example-design.md into GitHub issues`.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autosplit/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
# Claude Code only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autosplit/install.sh | sh -s -- --harness claude
# OpenCode only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autosplit/install.sh | sh -s -- --harness opencode
# Codex CLI only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autosplit/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autosplit
./install.sh --harness all
```

## What it does

- **Startup self-update** — Refreshes the installed `autosplit` copy from `main`
  before normal execution, using the same fail-open preflight as the other
  autospec skills.
- **Phase 0** — Verifies the current directory is a GitHub-backed git repo.
- **Phase 3** — Foreground subagent decomposes the selected spec into an EPIC
  umbrella plus N self-contained children pre-staged for 32B-class local LLMs.
- **Phase 3.5** — Reviews and labels the generated child issues with the
  `ctx:*` / `reasoning:*` rubric.

In existing spec mode, the skill verifies the selected spec is present on
`origin/main`, skips Phases 1–2, then runs Phase 3 and Phase 3.5 against that
spec. If multiple specs are available and no path is supplied, it asks which
spec to split before filing issues.

When Phase 3 completes the skill stops and prints:

```
Phase 3 complete. Run /autospec-run --profile <name> to begin implementation.
```

## Subagent model selection (Tier A only)

This skill is the existing-spec split shortcut. Its Phase 3 decomposition and
Phase 3.5 review-and-label subagents run on **Tier A** per `AGENTS.md`: top
model with extended/maximum thinking. Issue quality is the bottleneck, so we pay
for the top model here once rather than N times in cheap-implementer corrections
downstream.

The Tier B (cheaper + medium thinking) tier is **not used** by this skill; it's reserved for `autospec-run`'s implementation loop.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec`](../autospec/README.md) | The full pipeline (Phases 0–6) when you want one skill that plans **and** ships. |
| [`autospec-run`](../autospec-run/README.md) | Implementation half (Phases 4–6). Picks up where `autosplit` stops; supports `--profile <name>` filtering against `~/.autospec/model-profiles.yml`. |
| [`autospec-classify`](../autospec-classify/README.md) | Standalone retro-labeler for already-existing `auto-implement` issues; applies the Phase 3.5 `ctx:*` / `reasoning:*` rubric. |

## Self-update

Once installed, run the skill with the literal argument `update` to refresh in
place. The skill detects its harness, re-runs the canonical install one-liner
with `--update`, shows the diff, and stops without entering Phase 0:

```
/autosplit update
```

Or re-run the per-skill installer with `--update`:

```bash
./install.sh --harness all --update
```

## Usage

```
/autosplit split latest spec
/autosplit turn docs/specs/2026-05-01-example-design.md into GitHub issues
```

(Replace `/` with `@` for OpenCode, or invoke through Codex CLI as a slash command.)

## License

MIT — see the repository [LICENSE](../../LICENSE) file.

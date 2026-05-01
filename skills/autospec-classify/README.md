# autospec-classify

Standalone retro-labeler for the **autospec** suite. Walks every open
`auto-implement` issue in the current GitHub repo (excluding `type:tracker`),
classifies each with `ctx:*` / `reasoning:*` labels per the Phase 3.5 rubric,
and inserts a `## Model fit` block into the issue body. Defaults to labels-only;
`--apply-boards` opts into board assignment from `~/.autospec/project-map.yml`.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-classify/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-classify/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-classify/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-classify/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-classify
./install.sh --harness all
```

## Invocation

```
/autospec-classify [--issues N,N,N] [--label <label>] [--apply-boards] [--dry-run]
```

| Flag | Default | Behavior |
|---|---|---|
| `--issues N,N,N` | (all open) | Restrict to specific issue numbers. |
| `--label <label>` | `auto-implement` | Restrict to issues with the given label. |
| `--apply-boards` | off | Also assign issues to GitHub Projects per `~/.autospec/project-map.yml`. |
| `--dry-run` | off | Print proposed labels and `## Model fit` blocks without calling `gh issue edit`. |

The skill is **idempotent**: running twice on the same issue produces the same
labels and the same `## Model fit` block (delimited by HTML comment markers so
the previous block is replaced in place rather than stacked).

## What `--apply-boards` does (today vs. when B3 lands)

Today, `--apply-boards` is a documented **no-op** that prints:

```
--apply-boards: project-map reader not yet implemented (lands in B3); skipping board assignment for issue <N>.
```

When **PR B3** (issue #16) lands, the project-map reader replaces this
placeholder. The eventual config file:

```yaml
# ~/.autospec/project-map.yml
multi_match: union          # or first
mappings:
  ctx:32k: 7
  ctx:64k: 7
  ctx:120k: 12
  reasoning:deep: 12
```

For each issue's labels, look up project numbers and call
`gh project item-add <project_number> --owner <owner> --url <issue-url>`.
With `multi_match: union`, all matching projects get the issue.

## Rubric (`ctx:*` and `reasoning:*`)

See the [SKILL.md](./SKILL.md) Rubric section for the full table. Quick summary:

- `ctx:32k` — one canonical table or shell script; ≤3 staged files.
- `ctx:64k` — multi-file change; 4–7 staged files.
- `ctx:120k` — cross-skill or cross-package; 8+ files.

- `reasoning:shallow` — copy/rename/transcribe.
- `reasoning:medium` — template-following with judgment calls.
- `reasoning:deep` — novel design choices.

Default for issues lacking signal: `ctx:64k`, `reasoning:medium`.

## Subagent model selection (Tier A only)

The per-issue review/label subagent dispatched by this skill runs on **Tier A** per `AGENTS.md`: top model with extended/maximum thinking. Review-and-label is spec-adjacent work — the `ctx:*` / `reasoning:*` labels and `## Model fit` block this skill writes drive every downstream `autospec-run --profile` decision, so spec-tier judgment is warranted.

The Tier B (cheaper + medium thinking) tier is **not used** by this skill; it's reserved for `autospec-run`'s implementation loop.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec`](../autospec/README.md) | The full pipeline (Phases 0–6) — Phase 3.5 inside that pipeline calls into the same rubric this skill applies retroactively. |
| [`autospec-define`](../autospec-define/README.md) | Planning half (Phases 0–3). |
| [`autospec-run`](../autospec-run/README.md) | Implementation half (Phases 4–6) — consumes the `ctx:*` / `reasoning:*` labels this skill backfills. |

## Self-update

Once installed, run the skill with the literal argument `update` to refresh in
place. The skill detects its harness, re-runs the canonical install one-liner
with `--update`, shows the diff, and stops without classifying any issue:

```
/autospec-classify update
```

Or re-run the per-skill installer with `--update`:

```bash
./install.sh --harness all --update
```

## License

MIT — see the repository [LICENSE](../../LICENSE) file.

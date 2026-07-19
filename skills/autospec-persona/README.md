# autospec-persona

Calibration-interview skill for the **autospec** suite. A Tier-A agent reads the
current repo (`docs/specs/`, `docs/memory/`, `AGENTS.md`, the autonomy charter,
recent commits, open issues, and the main code areas), surfaces the real decision
tensions, and turns them into **≤50 repo-grounded questions** (a hard cap) asked
in themed `AskUserQuestion` batches. The operator's answers persist to
`~/.autospec/operator-persona.answers.json` and the interview is fully
**resumable** — re-running continues at the next pending batch and never re-asks a
`done` batch.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-persona/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-persona/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-persona/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-persona/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-persona
./install.sh --harness all
```

## Invocation

```
/autospec-persona [--reset] [--max-questions N] [--dry-run]
```

| Flag | Default | Behavior |
|---|---|---|
| `--reset` | off | Discard the existing answers file and start fresh at `next_batch=0`. |
| `--max-questions N` | 50 | Lower the budget below the hard cap of 50 (values above 50 are clamped). |
| `--dry-run` | off | Print themed batches + calibration predictions; do not call `AskUserQuestion` or write the file. |

## Answers file

The interview reads and writes `~/.autospec/operator-persona.answers.json`
(version 1):

```json
{
  "version": 1,
  "batches": [
    { "id": "merge-autonomy", "status": "done",
      "questions": [ { "q": "...", "choice": "auto-merge", "free": null } ] }
  ],
  "next_batch": 1
}
```

- **Resumable:** `next_batch` is the index of the first non-`done` batch.
  Re-running resumes there and never re-asks a `done` batch.
- **Calibration:** a subset of multiple-choice questions are *calibration*
  questions whose answer the agent infers up-front. Agreement % = matched ÷ total
  multiple-choice calibration questions × 100. Free-text answers are excluded from
  the percentage; disagreements correct the inferred model.

## Self-update

Run the skill with the literal argument `update` to refresh in place:

```
/autospec-persona update
```

Or re-run the installer with `--update`:

```bash
./install.sh --harness all --update
```

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec`](../autospec/README.md) | The full pipeline (Phases 0–6). |
| [`autospec-define`](../autospec-define/README.md) | Planning half (Phases 0–3). |
| [`autospec-run`](../autospec-run/README.md) | Implementation half (Phases 4–6). |

## License

MIT — see the repository [LICENSE](../../LICENSE) file.

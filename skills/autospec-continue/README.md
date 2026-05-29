# autospec-continue

Bridge conversational recommendations into autospec execution. A new top-level
skill `/autospec-continue` (alias `/continue`) that extracts the actionable
recommendation from the most recent assistant message in the current
conversation, pipes it through `/autospec-refine` (the 4-lens refinement
pipeline shipped via PR #678), and hands off to `/autospec --autonomous` for
end-to-end implementation.

The point: any time the model just said "here's what to do next", the operator
types `/continue` and the model's own suggestion becomes an autospec loop, with
no copy-paste and no manual prompt rewriting.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-continue/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-continue/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-continue/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-continue/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-continue
./install.sh --harness all
```

## Invocation

```
/autospec-continue [--skip-refine] [--ask-confirm] [--lens-mode deterministic|llm]
                   [--from-message <path>]
```

- `--skip-refine` — bypass `/autospec-refine`; hand the extracted prompt
  directly to `/autospec --autonomous`.
- `--ask-confirm` — after refine but before handoff, surface the refined
  prompt and ask the operator to approve. Defaults to auto-execute.
- `--lens-mode deterministic|llm` — passthrough to `/autospec-refine`'s
  same flag. Default: matches refine's auto-detect.
- `--from-message <path>` — read the source message from a file instead of
  the harness's "last assistant turn". Test/integration path; not the
  primary use case.

## Extraction matchers (priority order)

1. **Section headings** — `## Next steps`, `## What to do next`,
   `## Remaining work`, `## Open blockers`, `## Suggestions`,
   `## Proposed follow-ups` (case-insensitive).
2. **Fenced blocks** — ` ```autospec-next ` or ` ```next-prompt ` fenced
   code blocks. Body extracted verbatim.
3. **Numbered lists with action verbs** — lists where >=50% of items start
   with imperative verbs (`fix`, `add`, `implement`, `update`, `review`,
   `refactor`, `ship`, `merge`, etc.).
4. **"You should / I suggest" patterns** — sentences starting with `you
   should`, `I suggest`, `I recommend`, `next step is`, `the next thing to
   do is`. That sentence plus the following paragraph.

## Safety

- **Prompt-injection guard** — extracted blocks containing `ignore previous`,
  `disregard prior`, `you are now`, `system prompt`, or leading shell
  metacharacters are rejected with
  `code_health:continue_injection_detected`.
- **Rate limiting** — `~/.autospec/continue-history.json` records source +
  extracted prompt hashes. Duplicate source within 60s →
  `code_health:continue_recent_duplicate`. Same extraction 3x in 60min →
  `code_health:continue_oscillation`.
- **Autonomy gate** — the autonomy guardrails (`scripts/autospec-autonomy-gate.sh`)
  apply downstream regardless of how the prompt got into the pipeline.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec`](../autospec/README.md) | Full pipeline (Phases 0–6) for plan + ship. |
| [`autospec-refine`](../autospec-refine/README.md) | N-round prompt refinement; consumes the extracted recommendation. |
| [`autospec-run`](../autospec-run/README.md) | Implementation half (Phases 4–6). |

## Self-update

Run the skill with the literal argument `update` to refresh in place:

```
/autospec-continue update
```

Or re-run the installer with `--update`:

```bash
./install.sh --harness all --update
```

## Stop

Halt a running continuation loop:

```
/autospec-continue stop --graceful   # finish current iteration, then exit
/autospec-continue stop --immediate  # abort at next iteration boundary
```

## License

MIT — see the repository [LICENSE](../../LICENSE) file.

# autospec-refine

N-round prompt refinement with repo-grounded lenses. Takes an operator-supplied
prompt (or feature request) and iteratively refines it over N rounds, grounding
each round in current repo knowledge (`AGENTS.md`, recent specs, recent
commits, tagged memory). After operator approval, hands off the final refined
prompt to `/autospec --autonomous` for end-to-end implementation.

Goal: better prompts → better code, fewer mid-implementation course
corrections, fewer rounds of operator feedback.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-refine/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-refine/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-refine/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-refine/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-refine
./install.sh --harness all
```

## Invocation

```
/autospec-refine "<initial prompt>" [--rounds N] [--from-file <path>] \
                 [--interactive | --autonomous (default) | --dry-run] \
                 [--lenses <comma-list>] [--output <path>] \
                 [--continue [--max-iterations N]]
```

- `--rounds N` — number of refinement passes. Default `3`. Max `10`.
- `--from-file <path>` — read the initial prompt from a file.
- `--autonomous` (default) — hand off to `/autospec --autonomous "<refined>"`.
- `--interactive` — hand off to plain `/autospec "<refined>"`.
- `--dry-run` — produce the artifact only, do not hand off.
- `--lenses repo,clarity,sizing,adversarial` — override the default lens order.
- `--output <path>` — write the final refined prompt to a file.
- `--continue` — continuous iteration mode (refine → handoff → execute →
  harvest-report → re-refine, bounded by termination rules).

## Lenses

Four registered lenses, applied in order one per round:

1. **`repo-grounding`** — replace generic verbs with project-specific file
   paths, conventions, lockstep, TDD, no-mock, conventional commits.
2. **`clarity-ac`** — extract implicit acceptance criteria into `- [ ]`
   checkboxes; disambiguate hedging language.
3. **`sizing`** — enforce small-LLM sizing caps (≤400 words, ≤3 files, ≤30
   outline lines); split oversize prompts into parent+child fragments.
4. **`adversarial`** — apply the autospec-qa 20-question critical-question
   checklist.

## Artifacts

- `.autospec/refinements/<slug>-<ts>.json` — structured per-round record.
- `.autospec/refinements/<slug>-<ts>.md` — human-readable report.
- `schemas/autospec-refinement.schema.json` — schema validated by
  `autospec validate`.

For continuous mode:

- `.autospec/refinements/<slug>-loop.json` — per-iteration record.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec`](../autospec/README.md) | Full pipeline (Phases 0–6) for plan + ship. |
| [`autospec-define`](../autospec-define/README.md) | Planning half (Phases 0–3). Receives refined prompts from `autospec-refine`. |
| [`autospec-run`](../autospec-run/README.md) | Implementation half (Phases 4–6). |

## Self-update

Run the skill with the literal argument `update` to refresh in place:

```
/autospec-refine update
```

Or re-run the installer with `--update`:

```bash
./install.sh --harness all --update
```

## Stop

Halt a running refinement loop:

```
/autospec-refine stop --graceful   # finish current iteration, then exit
/autospec-refine stop --immediate  # abort at next iteration boundary
```

## License

MIT — see the repository [LICENSE](../../LICENSE) file.

# autospec-loop

Harness-neutral, autospec-family **refine-then-goal-conditioned execution
loop**. Turns an operator request to "run something in a loop until a goal is
reached" into (1) an interactive **refine-until-`go`** gate that freezes a
testable contract, then (2) a **goal-conditioned loop** that re-dispatches the
refined prompt with carried state each iteration until a deterministic CHECK —
or, failing that, an independent verifier — confirms the goal is met.

It stops on the **goal**, not a clock, and ships conservative guardrails
(max-iters cap, no-progress halt, shared `stop.flag`, pre-loop short-circuit)
so it never spins unbounded on small-context local models.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

> **Status:** scaffold. This issue ships the lock-step trio with the 7 required
> structural sections plus packaging. Phase 1 gate logic, Phase 2 loop logic,
> trigger wording, the `scripts/loop-state.sh` / `loop-check.sh` helpers, and
> `validate-autospec-loop.sh` land in follow-up child issues.

## autospec-loop vs native `/loop`

| Intent | Skill |
| --- | --- |
| "loop until the build passes", "keep going until done", "run X in a loop until Y" | **autospec-loop** (goal-conditioned) |
| "loop every 5m", "/loop 5m /foo", "poll every N minutes" | native `/loop` (interval) |

Ambiguous bare "do a loop" with no interval and no goal is claimed by
autospec-loop; the refine gate's first question is goal-or-interval, and an
interval answer redirects to native `/loop`.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-loop/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-loop/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-loop/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-loop/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-loop
./install.sh --harness all
```

## Invocation

```
/autospec-loop "<request>" [--goal '<criterion>'] [--check '<cmd>'] \
               [--max-iters N] [--no-progress K] [--yes]
```

- `--goal '<criterion>'` — supply the testable GOAL explicitly; skip inference.
- `--check '<cmd>'` — supply the deterministic CHECK (shell command); skip
  inference. Exit 0 = met, non-zero = not-met.
- `--max-iters N` — override the iteration cap (default 25).
- `--no-progress K` — override the consecutive-no-progress halt (default 3).
- `--yes` — skip the show-and-explain gate (autonomous; off by default, since
  the gate is the core requested behaviour).

## Required structural sections

Every harness body (`SKILL.md` / `codex/prompt.md` / `opencode/agent.md`)
carries these 7 sections in order, so the decomposer and the repo validator
recognise it as a conformant autospec skill:

1. **Startup self-update** — canonical self-update block, `SKILL_NAME=autospec-loop`.
2. **Self-update mode** — pure-prose dispatch on `^\s*update\s*$`.
3. **Model tier** — AGENTS.md Tier A/B + harness-detection + adapter row.
4. **Trigger disambiguation** — goal-intent vs interval (`/loop`) split.
5. **Phase 1 — refine-until-go gate**.
6. **Phase 2 — goal-conditioned loop**.
7. **Guardrails** — max-iters, no-progress, stop.flag, pre-loop short-circuit.

## State file

`~/.autospec/loop/<repo-slug>/loop-state.json` (path-scoped slug avoids
cross-repo collision; atomic `tmp`+`mv` writes). Schema: 1.

## Self-update

```
/autospec-loop update
```

Or re-run the installer with `--update`:

```bash
./install.sh --harness all --update
```

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-refine`](../autospec-refine/README.md) | The refinement rounds reused by Phase 1's gate. |
| [`autospec-stop`](../autospec-stop/README.md) | Writes the shared `~/.autospec/stop.flag` the loop honours. |
| [`autospec`](../autospec/README.md) | Full plan-and-ship pipeline. |

## License

MIT — see the repository [LICENSE](../../LICENSE) file.

# autospec-explore-ledger

Operator-facing surface for the **autospec-explore** outcome ledger — the
recursive-self-improvement (RSI) memory that closes the explore loop. The
explore loop files feature proposals from 6 research sources; this ledger
records each proposal's eventual outcome, derives dynamic per-source ranking
weights, and regenerates a human-readable learnings memo. This skill is a thin,
deterministic wrapper over three shell scripts — it never files issues or runs
the loop.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-explore-ledger/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-explore-ledger/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-explore-ledger/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-explore-ledger/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-explore-ledger
./install.sh --harness all
```

## What it does

The autospec-explore loop files feature proposals from 6 research sources
(spec/code gaps, prior reports, codebase signals, open issues, repo source
analysis, competitor research). The ledger records each filed proposal's
eventual outcome — `merged_clean`, `qa_failed`, `reverted`, `stalled`, or
`abandoned` — turning explore from an open loop into a closed, self-improving
one. Sources whose proposals ship clean in **this** repo gain ranking weight;
sources with repeat failures lose it.

## Subcommands

```
/autospec-explore-ledger <subcommand> [options]
```

| Subcommand | Underlying script | Purpose |
|---|---|---|
| `show [--source X]` | `explore-ledger.sh --show` | Inspect recorded outcomes (optionally for one source). |
| `stats` | `explore-ledger.sh --stats` | Per-source filed / clean / failed counts. |
| `rebuild [--repo R]` | `explore-ledger.sh --rebuild` | Reconstruct the ledger from GitHub issue/PR history. |
| `weights` | `explore-source-weights.sh --json --explain` | Show current dynamic source weights and the math behind them. |
| `learnings` | `explore-learnings.sh` | (Re)generate `.autospec/explore-learnings.md` and print it. |

With no subcommand the skill defaults to `stats`. Scripts resolve from
`${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/`.

## Where the ledger lives

The ledger is an append-only JSONL file at `.autospec/explore-ledger.jsonl`
(gitignored runtime memory; override with `AUTOSPEC_EXPLORE_LEDGER`). Outcomes
are recorded as new entries rather than rewrites; readers take the latest entry
per proposal. The derived learnings memo lives at
`.autospec/explore-learnings.md`.

`rebuild` reconstructs the ledger from GitHub history. When a live ledger already
exists, the reconstruction is written to `<ledger>.rebuilt` instead of clobbering
it, so the operator can diff and promote it.

## How it powers dynamic weighting + learnings

1. **File** — a researcher files a proposal issue carrying an
   `<!-- explore-ledger ... -->` marker in its body; explore records a `pending`
   ledger entry keyed on the issue.
2. **Drain** — `/autospec-run` implements the issue and opens a PR against the
   sandbox branch.
3. **Resolve** — when the PR merges, reverts, fails QA, stalls, or is abandoned,
   the outcome is appended as a new ledger entry.
4. **Stats → weights** — `stats` aggregates per-source counts; `weights` turns
   them into a dynamic Bayesian per-source weight that biases the next round's
   proposal ranking.
5. **Learnings** — `learnings` distills the same history into a human-readable
   memo.

## Marker grammar

Filed proposals carry an HTML-comment marker in the issue body:

```
<!-- explore-ledger source=<source-name> round=<n> ... -->
```

`rebuild` parses these markers from GitHub issue bodies to reconstruct the
ledger when the local JSONL is lost.

## Best-effort by design

All ledger operations are best-effort: a missing or empty ledger yields empty
stats/weights and an empty learnings memo, never a hard failure. The ledger is
runtime memory — losing it costs only the accumulated weighting signal, which
`rebuild` can reconstruct from GitHub history.

## Self-update

Once installed, run the skill with the literal argument `update` to refresh in
place across all harnesses, or re-run the per-skill installer:

```bash
./install.sh --harness all --update
```

## License

MIT — see the repository [LICENSE](../../LICENSE) file.

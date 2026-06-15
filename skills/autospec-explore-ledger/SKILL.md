---
name: autospec-explore-ledger
description: Use when the user wants to inspect, rebuild, or act on the autospec-explore outcome ledger — the recursive-self-improvement memory that records which research sources actually ship clean PRs, derives dynamic source weights, and produces a learnings memo. Subcommands — show, stats, rebuild, weights, learnings.
---

# autospec-explore-ledger workflow (harness-neutral)

Operator-facing surface for the **autospec-explore** outcome ledger — the
recursive-self-improvement (RSI) memory that closes the explore loop. The
explore loop files feature proposals from research sources; this ledger records
each proposal's eventual outcome, turns those outcomes into per-source stats,
derives dynamic Bayesian source weights, and regenerates a learnings memo. This
skill is a thin, deterministic wrapper over three shell scripts: it inspects,
rebuilds, and acts on that memory — it never files issues or runs the loop.

Manage your own context — never exceed 60%. These subcommands are deterministic
shell calls; run them inline and report their output. Delegate only if a
harness supports it and the output is large.

## Self-update mode

If the argument matches the regex `^\s*update\s*$` (case-insensitive,
whitespace-padded), this skill enters self-update mode and does not run any
subcommand. This section is pure prose: never interpolate or shell out the
operator's argument text.

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-explore-ledger/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-explore-ledger.md`
   - Codex CLI:   `~/.codex/prompts/autospec-explore-ledger.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not run any subcommand. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-explore-ledger found; run install.sh first.` and exit.

## Model tier

Tier A/B model resolution and the harness-detection protocol are inherited
verbatim from `AGENTS.md` (`## Subagent model selection (two-tier, cost-aware)`
and `## Subagent vs inline decision matrix`). This skill is deterministic-first:
every subcommand is a shell call with no LLM dispatch. Only a free-form summary
of a large `show`/`stats` dump (if the operator asks for narrative) escalates to
a single Tier B pass — never Tier A.

> **Model tier:** subcommands run deterministically with no dispatch; an optional narrative summary of ledger output runs at TIER_B (implementation work), resolved at startup per the harness detection below.

### Required capabilities & harness adapter

This workflow assumes the following capabilities. Map each to your harness's
actual tool; if a capability is missing, use the listed fallback.

| Capability                  | Claude Code                             | OpenCode                              | Codex CLI                                | Fallback if missing                              |
|-----------------------------|-----------------------------------------|---------------------------------------|------------------------------------------|--------------------------------------------------|
| Run a shell command         | `Bash`                                  | shell tool                            | shell                                    | none — this skill requires a shell               |
| Optional narrative summary  | `Agent` (subagent_type=general-purpose) | `task` agent, await output            | summarize inline (no subagents)          | Summarize in-thread (more context cost)          |
| Ask the user a question     | `AskUserQuestion`                       | inline prompt                         | inline prompt                            | Ask in the response and wait for the next turn   |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
| Subagent dispatch policy    | per AGENTS.md decision matrix           | per AGENTS.md decision matrix         | per AGENTS.md decision matrix            | inline with main-session token cost              |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in
the repo root — recognized by Claude Code, OpenCode, and Codex.

## Harness detection (run once at skill start, before any subcommand)

Detect your harness by checking available tools before any dispatch:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)
2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning
3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; work runs inline.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness, silently retry
the same subagent dispatch with `TIER_A`. Preserve parent context on retry.
Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run.

## What this is

The autospec-explore loop files feature proposals from 6 research sources
(spec/code gaps, prior reports, codebase signals, open issues, repo source
analysis, competitor research). This ledger records each filed proposal's
eventual outcome — `merged_clean`, `qa_failed`, `reverted`, `stalled`, or
`abandoned` — turning explore from an open loop into a closed, self-improving
one. Sources whose proposals ship clean in THIS repo gain ranking weight;
sources with repeat failures lose it. The ledger is append-only: outcomes are
recorded as new entries, and readers take the latest entry per proposal.

## Subcommands

Resolve the scripts at `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/<script>`.

| Subcommand | Runs | Purpose |
|---|---|---|
| `show [--source X]` | `explore-ledger.sh --show` | inspect recorded outcomes (optionally for one source) |
| `stats` | `explore-ledger.sh --stats` | per-source filed/clean/failed counts |
| `rebuild [--repo R]` | `explore-ledger.sh --rebuild` | reconstruct the ledger from GitHub issue/PR history (writes `<ledger>.rebuilt` if a live ledger exists) |
| `weights` | `explore-source-weights.sh --json --explain` | show current dynamic source weights (and the math) |
| `learnings` | `explore-learnings.sh` | (re)generate `.autospec/explore-learnings.md` and print it |

- `show [--source X]` — run `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/explore-ledger.sh --show` (append `--source X` to filter to one source). Print the recorded outcomes.
- `stats` — run `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/explore-ledger.sh --stats`. Print the per-source filed/clean/failed table.
- `rebuild [--repo R]` — run `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/explore-ledger.sh --rebuild` (append `--repo R` to target a specific repo). If a live ledger already exists, the script writes the reconstruction to `<ledger>.rebuilt` rather than clobbering it; report that path so the operator can diff and promote it.
- `weights` — run `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/explore-source-weights.sh --json --explain`. Print the JSON weight map and the per-source explanation lines.
- `learnings` — run `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/explore-learnings.sh`. It (re)derives `.autospec/explore-learnings.md`; print the resulting memo path and contents.

If the operator gives no subcommand, default to `stats` (the cheapest at-a-glance view) and list the other subcommands.

## How the loop closes

1. **File** — a researcher files a proposal issue carrying a `<!-- explore-ledger ... -->` marker in its body; explore records a `pending` ledger entry keyed on the issue.
2. **Drain** — `/autospec-run` implements the issue and opens a PR against the sandbox branch.
3. **Resolve** — when the PR merges, reverts, fails QA, stalls, or is abandoned, the outcome is recorded as a new append-only entry (`--update-outcome`).
4. **Stats** — `stats` aggregates filed/clean/failed counts per source from the latest entry per proposal.
5. **Weights** — `weights` turns those counts into a dynamic Bayesian per-source weight that biases the NEXT round's proposal ranking.
6. **Learnings** — `learnings` distills the same history into `.autospec/explore-learnings.md` for human review.

The ledger lives at `.autospec/explore-ledger.jsonl` (gitignored runtime memory;
override with `AUTOSPEC_EXPLORE_LEDGER`). All ledger operations are best-effort:
a missing or empty ledger yields empty stats/weights, never a hard failure.

## Finalization

Print which subcommand ran and the relevant path(s):

- `show` / `stats` / `weights` — the ledger path read (`.autospec/explore-ledger.jsonl`).
- `rebuild` — the written path (the live ledger, or `<ledger>.rebuilt` when a live ledger was preserved).
- `learnings` — the memo path (`.autospec/explore-learnings.md`).

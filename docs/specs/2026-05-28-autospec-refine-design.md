# autospec-refine — N-round prompt refinement with repo-grounded lenses

## Summary

A new top-level skill `/autospec-refine` that takes an operator-supplied prompt
(or feature request) and iteratively refines it over N rounds, grounding each
round in current repo knowledge. After operator approval, hands off the final
refined prompt to `/autospec --autonomous` for end-to-end implementation.

Goal: better prompts → better code, fewer mid-implementation course corrections,
fewer rounds of operator feedback. Today operators either feed raw requests to
autospec (under-specified) or hand-craft long prompts manually (high cost).
`autospec-refine` closes that gap by automating the prompt-improvement loop
that operators currently do in their heads.

## Team personality

- **Selected team:** Core product engineering — product manager, architect,
  backend developer, test engineer, technical writer.
- **Why this team fits:** developer tooling that touches LLM prompt quality;
  needs balanced product judgment (when to stop refining), engineering rigor
  (helper-script + bats), and writing craft (the refined prompts ARE the
  output).
- **Risks this team will notice:** runaway refinement loops, prompt bloat,
  unclear convergence signals, scope drift between rounds, brittle handoff to
  `/autospec`.
- **Carry into child issues:** every round must produce a measurable, named
  improvement; convergence and degradation detection are first-class; handoff
  is auditable.

## Review counter-team

- **Selected counter-team:** Security + maintainability — security reviewer,
  maintainer, regression-test engineer.
- **Why this counter-team:** the refinement engine reads `AGENTS.md`, recent
  specs, git log, and `~/.autospec/` memory — risks leaking repo internals,
  user-private memory, or stale secrets into prompts that may flow to remote
  LLMs. Maintainability angle catches knobs-creep (lenses, flags, rounds).
- **What this team should challenge:** what does the refined prompt contain
  that the operator would NOT want sent to a remote LLM? What happens when
  refinement reads stale memory? Are new flags justified or knob-creep?

## Architecture

`autospec-refine` is a new top-level skill that mirrors the existing
`autospec-define` and `autospec-qa` skill-family shape:

- `skills/autospec-refine/SKILL.md` — Claude Code adapter (authoritative).
- `skills/autospec-refine/codex/prompt.md` — Codex CLI mirror (lockstep).
- `skills/autospec-refine/opencode/agent.md` — OpenCode mirror (lockstep).
- `skills/autospec-refine/install.sh` — installer; mirrors existing skills.
- `skills/autospec-refine/uninstall.sh` — uninstaller; mirrors existing skills.
- `skills/autospec-refine/README.md` — usage doc.
- `scripts/refine-prompt.sh` — orchestrator: parses args, dispatches lenses,
  writes artifacts, performs handoff.
- `scripts/refine-render-overview.sh` — produces dual markdown + JSON artifact.

## Invocation

```
/autospec-refine "<initial prompt>" [--rounds N] [--from-file <path>] \
                 [--interactive | --autonomous (default) | --dry-run] \
                 [--lenses <comma-list>] [--output <path>]
```

- `--rounds N` — number of refinement passes. Default `3`. Max `10`. Rounds
  apply the named lenses in order; if `N` < number of lenses, the trailing
  lenses are skipped. If `N` > number of lenses, the adversarial lens repeats.
- `--from-file <path>` — read the initial prompt from a file instead of
  inline string. Mutually exclusive with positional prompt.
- `--autonomous` (default) — on operator approval, hand off to
  `/autospec --autonomous "<refined>"` (no-confirmation autospec, shipped via
  PR #664).
- `--interactive` — hand off to plain `/autospec "<refined>"` (full Phase 2
  brainstorm gate).
- `--dry-run` — produce the artifact only, do not hand off.
- `--lenses repo,clarity,sizing,adversarial` — override the default lens
  order. Comma-separated, drawn from the registered lens names.
- `--output <path>` — write the final refined prompt to a file (in addition
  to the `.autospec/refinements/` artifact).

## Refinement lenses

Four lenses, applied in order one per round:

1. **`repo-grounding`** — read `AGENTS.md`, recent `docs/specs/**` (last 30
   days), `git log --since=7d`, and tag-matched `~/.autospec/projects/*/memory/feedback_*.md`.
   Replace generic verbs with project-specific file paths, conventions, and
   constraints (lockstep, TDD, no-mock, conventional commits, sizing caps).
2. **`clarity-ac`** — extract implicit acceptance criteria and convert prose
   into `- [ ]` checkbox list. Disambiguate hedging language (`should
   probably`, `might`, `could try`). Name unambiguous test commands.
3. **`sizing`** — enforce small-LLM execution caps from the autospec
   decomposer rule (body ≤400 words, ≤3 files per child issue, ≤30 lines of
   implementation outline). If the refined prompt would produce a child that
   exceeds caps, split into a parent + child sequence and emit linked prompt
   fragments.
4. **`adversarial`** — critical-question pass. Apply the 20-question
   checklist from `autospec-qa`. For each high-risk answer that's actionable
   inside the scope, add a test requirement or scope clarification.

Operators may override the default set with `--lenses`. Custom lens registry
is out of scope for v1.

## Convergence and degradation

- **Convergence** — if round N's refined prompt is byte-identical to round
  N-1's, exit early with status `converged` and final round = N-1.
- **Degradation** — if round N's prompt is shorter than round N-1's by more
  than 25% by word count, flag `degraded:<lens>` and surface to operator.
  This catches lens implementations that over-aggressively prune scope.
- **Round cap** — hard limit `AUTOSPEC_REFINE_MAX_ROUNDS=10`. Beyond this,
  exit with `round_cap_reached`.

## Repo context scope

| Source | Read pattern |
|---|---|
| `AGENTS.md` | Always read in full. Highest-signal source. |
| `docs/specs/**/*.md` | Last 30 days by file mtime + commit date. Cap at 5 specs to avoid prompt bloat. |
| `git log --since=7d --oneline` + per-commit `git show --stat` | Last 7 days. Use to surface recently-changed files the prompt might intersect. |
| `~/.autospec/projects/*/memory/feedback_*.md` | Keyword match against the prompt. Cap at 5 most-relevant entries. |

The orchestrator MUST NOT read `.env`, `.git/`, files under `node_modules/`,
or any path matching `*credential*`, `*secret*`, `*.pem`, `*.key`. Path
allowlist is enforced in `refine-prompt.sh`.

## Data model

`.autospec/refinements/<slug>-<ISO-timestamp>.json`:

```json
{
  "original_prompt": "...",
  "rounds": [
    {
      "round_number": 1,
      "lens": "repo-grounding",
      "sources_used": ["AGENTS.md", "docs/specs/2026-05-28-foo.md"],
      "refined_prompt": "...",
      "diff_summary": "added paths/conventions/lockstep refs",
      "word_count_delta": 87,
      "reasoning": "..."
    }
  ],
  "final_prompt": "...",
  "status": "approved|converged|degraded|round_cap_reached|aborted",
  "metadata": {
    "head_sha": "...",
    "timestamp": "...",
    "rounds_requested": 3,
    "rounds_executed": 3,
    "converged_early": false,
    "degraded_rounds": [],
    "handoff_target": "/autospec --autonomous",
    "handoff_executed": true
  }
}
```

`.autospec/refinements/<slug>-<ISO-timestamp>.md` — human-readable: original
prompt → per-round headings with diff blocks → final prompt → handoff record.

A JSON schema lives at `schemas/autospec-refinement.schema.json` and is
validated by `scripts/validate.sh`.

## Error handling

- Empty prompt → usage error, exit 2.
- LLM call fails (rate limit, auth, network) → retry once with backoff, then
  surface to operator and write partial artifact.
- Repo context insufficient (no `AGENTS.md`, no specs) → log warning, use
  generic refinements, mark `metadata.context_sparse: true`.
- Forbidden path access attempt (the allowlist enforced) → fail loudly with
  `code_health:refine_path_violation`; do not silently strip.
- Handoff target unavailable (e.g. `/autospec --autonomous` missing) → write
  artifact, print the refined prompt to stdout, exit with clear message.

## Testing

- `tests/refine/test_refine_orchestrator.bats`:
  - happy path: 3-round refinement with all 4 lenses → artifact written, JSON
    schema-valid, rounds populated.
  - convergence: round 2 == round 1 → early exit, status `converged`.
  - degradation: synthetic round that drops 30% of words → flag emitted.
  - round cap: `--rounds 15` → capped at 10, status `round_cap_reached`.
  - context sparse: no `AGENTS.md` + no specs → completes with warning.
  - forbidden path: lens tries to read `.env` → exits with
    `code_health:refine_path_violation`.
- `tests/refine/test_refine_lenses.bats`: per-lens isolation tests, one
  fixture each (4 tests).
- `tests/refine/test_refine_overview.bats`: dual-artifact renderer produces
  valid markdown + JSON.
- `tests/refine/test_refine_handoff.bats`: `--autonomous`, `--interactive`,
  `--dry-run` each route correctly (mocked `gh` / `claude` invocations).

## Acceptance

- New skill family `autospec-refine` lives in `skills/autospec-refine/` and
  passes `check_lockstep`.
- `scripts/refine-prompt.sh` exists, executable, `bash -n` clean.
- `scripts/refine-render-overview.sh` exists, executable, `bash -n` clean.
- `schemas/autospec-refinement.schema.json` exists and is JSON-schema valid.
- `scripts/validate.sh` gains `check_autospec_refine_contract()` enforcing
  lockstep + script presence + schema presence + bats suite.
- Bats fixtures pass (`tests/refine/test_refine_*.bats`).
- End-to-end: `bash scripts/refine-prompt.sh "fix login button" --rounds 3
  --dry-run` produces a refined prompt with measurable repo-grounding (cites
  at least one path from `AGENTS.md` or recent specs).

## Continuous-iteration mode (`--continue`)

`/autospec-refine --continue "<initial prompt>" [--max-iterations N]` runs the
refine → handoff → execute → harvest-report cycle in a loop. After each
`/autospec` run completes, the orchestrator reads the run's final report
(from `.autospec/run-summary.md` or the equivalent), extracts the "next
steps" / "blockers" / "remaining work" section, and uses THAT content as the
input prompt for the next refinement round.

Motivating example: a benchmark-improvement run reports `391/1000 exact
matches; out-of-sample check shows overfitting risk; next step needs a
deterministic ontology layer, not more one-family rules`. Without
`--continue`, the operator manually writes a new prompt for that next step.
With `--continue`, the orchestrator reads the report, identifies "deterministic
ontology layer" as the next prompt, refines it (the same 4 lenses), and
executes — until the loop terminates.

### Termination (any one)

- **Convergence — no next-steps content.** The harvested report contains no
  actionable "next steps", "blockers", "remaining work", or "what to do next"
  section, OR the section is empty / says "done" / "no further work".
- **Oscillation.** Iteration N+1's harvested prompt equals iteration N's
  (hashed by content). Auto-fix is going nowhere; loop exits with
  `oscillation_detected`.
- **Round cap.** `AUTOSPEC_REFINE_LOOP_MAX_ITERATIONS` (default 5).
- **Budget cap.** Tokens > `AUTOSPEC_REFINE_LOOP_TOKEN_CAP` (default 2M) or
  wall time > `AUTOSPEC_REFINE_LOOP_TIME_CAP` (default 6h).
- **Evidence-based stop.** The harvested report contains an explicit
  `STOP: <reason>` marker (e.g., `STOP: out-of-sample plateau evidence`).
  Operator-defined stop conditions.
- **Operator escape.** `~/.autospec/stop.flag` (graceful) or
  `~/.autospec/refine-loop-stop.flag` terminates at the next iteration
  boundary.

### Per-iteration record

Each loop iteration appends to `.autospec/refinements/<slug>-loop.json`:

```json
{
  "iteration": 2,
  "harvested_from_report": ".autospec/runs/2026-05-28T14:00Z/report.md",
  "harvested_prompt": "Implement a deterministic ChemOnt-aligned ontology classifier...",
  "refinement_artifact": ".autospec/refinements/<slug>-iter2-<ts>.json",
  "handoff_pr_count": 3,
  "handoff_pr_numbers": [672, 673, 674],
  "stop_reason": null
}
```

### Report harvest contract

The orchestrator reads the LAST autospec run's report and looks for, in order:

1. A section header matching `## Next steps`, `## What to do next`,
   `## Remaining work`, or `## Open blockers` (case-insensitive).
2. If absent, fenced code blocks tagged with `\`\`\`autospec-next` or
   `\`\`\`next-prompt`.
3. If absent, the report's `Out-of-sample`, `Stop condition`, or
   `Evidence-backed stop` sections are inspected. If they say "stop", the
   loop terminates with `evidence_based_stop`.
4. If none of the above are present → convergence (loop exits cleanly).

Autospec's existing final report (per `/autospec` Phase 6) gains the
canonical `## Next steps` section structure so the harvest is reliable. That
report-format change is part of Issue E.

### Safety guardrails

- The `--autonomous` mode safety guardrails (PR #664, `scripts/autospec-autonomy-gate.sh`)
  still apply on every loop iteration. Destructive-remote, out-of-scope,
  cost-cap checks run before each execute.
- The continuous loop inherits the autospec autonomy scope rules; it does
  NOT escalate privileges.
- Every iteration's per-PR merge still goes through the rebase-and-retest
  gate.

### Summary output

At loop end, print a Markdown table:

```
## /autospec-refine continuous loop summary

| Iter | Harvested from        | Refined prompt (first 60 chars) | PRs merged | Time | Status              |
|------|-----------------------|---------------------------------|-----------:|------|---------------------|
| 1    | (operator input)      | Fix all dashboards end-to-end…  |          6 | 28m  | next-steps found    |
| 2    | runs/…/report.md      | Add ChemOnt ontology layer…     |          4 | 41m  | next-steps found    |
| 3    | runs/…/report.md      | Validate against MassBank corpus|          2 | 19m  | convergence_clean   |

Final status: convergence_clean
Total PRs merged across loop: 12
```

Plus per-PR list across all iterations and the per-iteration JSON artifact.

## Out of scope (defer)

- Custom lens registry (operators registering their own lenses).
- Cross-repo refinement (the `--repo` flag — single-repo only for v1).
- Refinement quality metrics / scoring of how much a refined prompt improved.
- Auto-detection of "this prompt needs refinement" — always operator-invoked.
- Caching refined prompts across sessions.
- Integration with `/autospec-qa` heal loop as a refinement step (defer to a
  follow-up issue once both are stable).

## Decomposition into child issues

Aiming for 4 children plus an umbrella, sized per the small-LLM rule.

1. **Issue A — skill scaffold**: `skills/autospec-refine/` trio +
   `install.sh` + `uninstall.sh` + `README.md`. Includes the structural
   Self-update + Model tier + adapter row required by the autospec-discovery
   contract. Files: 6.
2. **Issue B — orchestrator + 4 lenses**: `scripts/refine-prompt.sh` with the
   four-lens dispatcher + per-lens helpers + `tests/refine/test_refine_lenses.bats`
   + `tests/refine/test_refine_orchestrator.bats`. Depends on A. Files: 3.
3. **Issue C — dual artifact renderer + schema**: `scripts/refine-render-overview.sh`
   + `schemas/autospec-refinement.schema.json` + `tests/refine/test_refine_overview.bats`.
   Depends on B. Files: 3.
4. **Issue D — handoff + validate.sh wiring + e2e**: handoff plumbing
   (`--autonomous` / `--interactive` / `--dry-run`) inside `refine-prompt.sh`,
   `check_autospec_refine_contract()` in `scripts/validate.sh`, and the
   `tests/refine/test_refine_handoff.bats` suite. Depends on A+B+C. Files: 2.
5. **Issue E — continuous-iteration mode (`--continue`)**: report-harvest
   logic in `refine-prompt.sh`, canonical `## Next steps` section format
   added to the autospec final report (`/autospec` Phase 6 prose update, in
   lockstep across the autospec trio), per-iteration JSON record schema,
   loop summary table renderer, and `tests/refine/test_refine_loop.bats`.
   Depends on A+B+C+D. Files: 3.

Total: 5 children + 1 umbrella. Each child is well under the autospec sizing
caps.

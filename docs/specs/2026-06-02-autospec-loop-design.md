# autospec-loop — interactive refine-then-goal-conditioned execution loop

- **Date:** 2026-06-02
- **Status:** Design (Phase 2)
- **Author:** berlinguyinca (brainstormed with Claude)
- **Tracker target:** `berlinguyinca/autospec`

## Problem statement

Operators frequently want to run a single task **repeatedly until a goal is
reached** — "keep fixing the build until it passes", "loop until all endpoints
return 200", "keep going until the migration is complete". Today the autospec
family offers no harness-neutral way to do this:

- **`autospec-refine`** improves a prompt over N rounds but then hands off to a
  **one-shot** `/autospec` implementation. It has no loop and no goal-conditioned
  stop.
- **Native `/loop`** (Claude Code only) re-runs a prompt on an **interval** or
  self-paced. It has **no refinement gate**, **no semantic goal stop** (it stops
  on count/time/operator judgement), and **does not exist on Codex or OpenCode**.
- **OMC `ralph`** loops until completion with a verifier, but is **not
  harness-neutral** and is **not part of the autospec family** (no self-update,
  no Tier A/B model resolution, no autospec stop.flag integration).

The gap: a **harness-neutral**, autospec-family skill that (1) **interactively
refines** a raw request into a frozen, operator-approved *contract*, then (2)
runs a **goal-conditioned loop** — re-dispatching the refined prompt with carried
state each iteration — until a deterministic check (or, failing that, an
independent verifier) confirms the goal is reached, with conservative guardrails
so it never spins unbounded.

## Goals / non-goals

**Goals**

- Interactive **refine-until-"go"** gate that *shows the refined prompt and
  explains what was understood*, and keeps iterating with the operator until the
  operator types the literal token `go`.
- **Goal-conditioned** termination: stop when the goal is verifiably met.
- **Harness-neutral**: no reliance on any scheduler (`ScheduleWakeup`, native
  `/loop`, cron). A synchronous, in-conversation loop. Works on Claude Code,
  OpenCode, and Codex CLI.
- **Conservative guardrails**: max-iterations cap + no-progress halt + shared
  `~/.autospec/stop.flag`.
- **Auto-invocation** by goal-intent loop phrasings, without cannibalising the
  native interval `/loop`.

**Non-goals**

- Interval/cron scheduling (delegate to native `/loop` when the intent is
  interval polling).
- Parallel/fan-out execution (single-lane loop; fleet/parallel work stays with
  `autospec-fleet` / `autospec-run`).
- Replacing `autospec-refine` (this skill *reuses* it for the refinement rounds).

## Design overview

A new top-level operator skill **`autospec-loop`**, shipped as the standard
multi-harness trio with lock-step-identical bodies:

```
skills/autospec-loop/
  SKILL.md            # Claude Code  (frontmatter: name + description)
  codex/prompt.md     # Codex CLI    (no frontmatter name; leading blank line)
  opencode/agent.md   # OpenCode     (frontmatter: description + mode: primary)
  README.md
  install.sh
  uninstall.sh
  scripts/
    loop-state.sh     # atomic read/update of loop-state.json (tmp+mv)
    loop-check.sh     # run the deterministic CHECK, normalise exit status
```

Per the **lock-step rule** (`CONTRIBUTING.md` / `AGENTS.md`), the three skill
bodies are byte-identical except frontmatter. Tier A/B model resolution and the
harness-detection protocol are inherited verbatim from `AGENTS.md`.

### Required structural sections (every harness body)

Each of `SKILL.md` / `codex/prompt.md` / `opencode/agent.md` MUST contain, in
this order, so the decomposer and `validate.sh` recognise it as a conformant
autospec skill:

1. **Startup self-update** — the canonical autospec self-update bash block with
   `SKILL_NAME=autospec-loop`.
2. **Self-update mode** — pure-prose dispatch on `^\s*update\s*$` (no shelling
   out of operator text; matches the "mode-dispatch must not shell user text"
   constraint).
3. **Model tier** — reference to AGENTS.md Tier A/B + the harness-detection
   protocol; the per-harness adapter row.
4. **Trigger disambiguation** — the goal-intent vs interval split (below).
5. **Phase 1 — refine-until-go gate** (below).
6. **Phase 2 — goal-conditioned loop** (below).
7. **Guardrails** — max-iters, no-progress, stop.flag, pre-loop short-circuit.

> **Decomposer note (per project memory `feedback_autospec_decomposer_gotchas`):**
> the **first** "create the autospec-loop skill" issue MUST itself carry these
> structural sections (Self-update + Model tier + adapter row) verbatim in its
> acceptance criteria, and `codex/prompt.md` MUST start with a leading blank
> line for lock-step. The decomposer should NOT apply the needs-autospec-template
> transform to these issues — the template is already specified here.

## Auto-invocation & the native `/loop` collision

The frontmatter `description` encodes **goal-intent disambiguation** so
auto-invocation routes correctly:

- **autospec-loop claims** goal/completion phrasings: *"loop until …"*, *"in a
  loop until …"*, *"keep going until done"*, *"do a loop"*, *"execute a loop"*,
  *"run X in a loop until Y"*, *"keep doing X until Y"*.
- **Native `/loop` keeps** bare interval phrasings: *"loop every 5m"*,
  *"/loop 5m /foo"*, *"poll every N minutes"*, *"keep running … on an interval"*.
- **Ambiguous** bare *"do a loop"* / *"execute a loop"* with no interval and no
  explicit goal → autospec-loop claims it, and the refine gate's **first
  question** is goal-or-interval. If the operator wants interval polling, the
  skill **redirects to native `/loop`** and exits without starting its own loop.

## Phase 1 — refine-until-"go" gate

1. **Trigger disambiguation** (above). If interval intent → redirect to native
   `/loop`, stop.
2. **Refinement rounds** — invoke **`autospec-refine --interactive`** to run the
   repo-grounded prompt-improvement rounds on the raw request (reuse upstream per
   the ROI-check rule; do not fork the lens logic).
3. **Contract extraction** — from the refined prompt, extract three artifacts:
   - **GOAL** — an explicit, testable success criterion (one or more sentences).
   - **CHECK** — a deterministic verifier (shell command + expected exit status,
     `grep`, or `gh` query) **when one is derivable**; otherwise `null`.
     Operator may supply it directly via `--check`.
   - **MODE** — `cumulative` (carry state between iterations; default) vs
     `polling` (stateless re-run of the same prompt).
4. **Show & explain** — present to the operator, in plain language:
   - the **refined prompt** verbatim;
   - **what was understood**: restated GOAL, the CHECK that will gate
     termination (or "no deterministic check — an independent verifier subagent
     will judge each iteration"), the MODE, and the guardrails;
   - the per-iteration plan.
5. **Clarify loop** — the operator corrects/clarifies; **re-run steps 2–4** until
   the operator types the literal unlock token **`go`** (case-insensitive; also
   accepts "go ahead" / "start"). `--yes` skips the gate (off by default, since
   the gate is the core requested behaviour).
6. **Freeze** the contract to the loop-state file (below).

## Phase 2 — goal-conditioned loop

**State file** at
`~/.autospec/loop/<repo-slug>/loop-state.json` (path-scoped slug to avoid the
known cross-repo collision in shared `~/.autospec`; atomic `tmp`+`mv` writes via
`loop-state.sh`):

```json
{
  "schema": 1,
  "goal": "<criterion>",
  "check": "<cmd or null>",
  "mode": "cumulative|polling",
  "max_iters": 25,
  "no_progress_K": 3,
  "iter": 0,
  "no_progress_count": 0,
  "progress": ["<iteration summaries>"],
  "last_result": "<text>",
  "done": false,
  "exit_reason": null
}
```

**Pre-loop short-circuit:** if CHECK is non-null, run it once before iterating;
if already satisfied → write `exit_reason="already-satisfied"`, report, stop. No
wasted iteration.

**Each iteration** (stop-gates evaluated *before* doing work):

1. **Halt gate** — stop with the matching `exit_reason` if any holds:
   `~/.autospec/stop.flag` exists (shared with `/autospec-stop`) →
   `"stopped-by-flag"`; `iter >= max_iters` → `"max-iters"`;
   `no_progress_count >= no_progress_K` → `"no-progress"`.
2. **Work** — dispatch a **subagent** (Tier B; **inline on Codex**, which has no
   subagents) with input = refined prompt + GOAL + `progress[]` (cumulative
   mode) or just the refined prompt (polling mode) + `last_result`. Instruct it
   to make forward progress toward GOAL and report what changed.
3. **Persist** — subagent outcome appended to `progress[]`, `last_result` set,
   via `loop-state.sh`.
4. **Goal eval** — if CHECK non-null, run `loop-check.sh` (deterministic,
   preferred). If CHECK is `null`, dispatch a **separate verifier subagent**
   (Tier B; Tier A for high-stakes goals) — **never the worker subagent**, to
   honour the no-self-approval rule — to judge `last_result`/repo state against
   GOAL and return met/not-met + reason.
5. **No-progress detection** — measurable change toward goal this iteration
   (CHECK output changed / files changed / non-empty progress delta)?
   No → `no_progress_count++`; yes → reset to 0.
6. **Converge** — goal met → `done=true`, `exit_reason="goal-met"`, break.
   Else `iter++`, continue.

**Exit report:** final status from `exit_reason` (goal-met / max-iters /
no-progress / stopped-by-flag / already-satisfied), iteration count, and the
`progress[]` log.

## Harness neutrality (core property)

- **No scheduler dependency.** Synchronous in-conversation loop — never
  `ScheduleWakeup`, native `/loop`, or cron. This is what makes it portable.
- **Subagents:** Claude Code `Agent` tool / OpenCode `task` tool; **Codex runs
  iterations inline** but executes identical loop logic and persists identical
  state.
- **CHECK and state I/O** are plain POSIX `bash` + `jq` — universal across
  harnesses.

## Flags

| Flag | Effect |
|------|--------|
| `--max-iters N` | Override iteration cap (default 25). |
| `--no-progress K` | Override consecutive-no-progress halt (default 3). |
| `--check '<cmd>'` | Supply the deterministic CHECK; skip inference. |
| `--goal '<criterion>'` | Supply GOAL explicitly; skip inference. |
| `--yes` | Skip the show-and-explain gate (autonomous; off by default). |
| `update` | Self-update mode (`^\s*update\s*$`). |

## Validation (repo has no language-level test runner)

Add `scripts/validate-autospec-loop.sh` (or extend `validate.sh`) that passes
after the change and covers, per the project's validation memories:

1. **Lock-step** — register the `autospec-loop` **trio** in the lock-step diff
   check (per `feedback_validate_sh_lockstep_duo_gap`: guard the full trio, and
   the SKILL.md+codex duo even if opencode is absent).
2. **Frontmatter** — `SKILL.md` parses `name` + `description`; `opencode/agent.md`
   parses `description` + `mode: primary`; `codex/prompt.md` has the **leading
   blank line** and no name frontmatter.
3. **Install scripts** — `bash -n` on `install.sh` / `uninstall.sh`.
4. **File presence** — trio + README + install/uninstall + `scripts/*.sh`.
5. **Named-content checks** (per `feedback_validate_sh_lockstep_checks`): assert
   each body contains the 7 required structural section headers; if a section is
   renamed, the check must be updated in the same change.
6. **Loop-logic guards** — assert the loop honours `stop.flag`, caps at
   `max_iters`, and uses a **separate** verifier subagent (grep the body for the
   no-self-approval clause).

## Risks & mitigations

| Risk | Mitigation |
|------|------------|
| Trigger collision with native `/loop` steals interval requests | Goal-intent description split + ambiguous→ask goal-or-interval + redirect path. |
| Infinite/expensive spin on a stuck task (small-model target) | Max-iters cap + no-progress halt + pre-loop short-circuit; all conservative defaults. |
| LLM self-assessment falsely declares "done" | Prefer deterministic CHECK; when absent, use an **independent** verifier subagent (not the worker). |
| Carried state bloats subagent context over long loops | State is a compact `progress[]` summary, not raw transcripts; subagent-per-iter keeps the orchestrator context flat. |
| Cross-repo state collision in shared `~/.autospec` | Path-scoped `<repo-slug>` state directory. |
| `set -e` / RETURN-trap footguns in scripts | Inline cleanup, `if/then/fi` for one-sided conditionals (per infra-gotcha memories). |

## Decomposition hint for `/autospec-define`

Suggested issue slicing (the first issue is the structural one):

1. **Scaffold the `autospec-loop` skill trio** — lock-step bodies with all 7
   required structural sections, Self-update + Model-tier + adapter row,
   README, install/uninstall. (First-issue structural requirement.)
2. **`loop-state.sh` + `loop-check.sh`** — atomic state I/O and CHECK runner.
3. **Phase 1 refine-until-go gate** — reuse `autospec-refine --interactive`,
   contract extraction, show-and-explain, `go` unlock.
4. **Phase 2 goal-conditioned loop** — iteration body, goal eval (CHECK /
   verifier), no-progress detection, exit report.
5. **Trigger disambiguation vs native `/loop`** — description wording + redirect.
6. **`validate-autospec-loop.sh`** — the validation coverage above.

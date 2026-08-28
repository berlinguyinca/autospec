---
name: autospec
description: Use when the user wants to ship a feature end-to-end across multiple commits — bootstraps repo if missing, brainstorms a design spec, decomposes into linked GitHub issues, or splits an existing spec into issues, then runs an autonomous implementation loop with auto-merge.
---

# autospec workflow (harness-neutral)

Take the following feature request and ship it through the full pipeline:
**bootstrap repo (if missing) → investigate → design → spec → decomposed GitHub issues → autonomous implementation with auto-merge → periodic status updates → final report.**

If the request asks to split, materialize, roadmap, decompose, or turn an
already-written spec into GitHub issues, use **Existing spec mode** below:
select a tracked `docs/specs/*.md` file, skip Phases 1-2, run Phase 3 and
Phase 3.5 against that spec, then continue to the Phase 3 pre-impl gate.

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it; do not investigate, write code, or design directly in the main conversation when a subagent can do it.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec -->

## Self-update mode

Decide this purely from the request text the harness handed you. Do NOT
shell out (no `grep`, `sed`, `[[ =~ ]]`, command substitution, etc.) to
test the user's free-form request — passing it through a shell is what
historically tripped harness permission engines (e.g. parse errors near
backtick/pipe characters in the user's prose). Read the request, normalize
it in your reasoning (collapse whitespace, trim, lowercase), and if the result is
exactly `update`, this skill enters self-update mode and does NOT run the
normal pipeline.

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec.md`
   - Codex CLI:   `~/.codex/prompts/autospec.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not enter Phase 0 / Phase 1 / any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec found; run install.sh first.` and exit.

## Stop mode

Apply the same read-and-normalize approach used for self-update mode (do
NOT shell out the user's request). If the normalized request is exactly
`stop`, or `stop` followed by one or more `--<word>` flags (examples:
`stop`, `stop --graceful`, `stop --immediate`, `stop --status`,
`stop --resume`, `stop --help`, `stop --flag`), this skill enters stop
mode and does NOT run the normal pipeline. When dispatching, pass any
`--<flag>` tokens the user provided as separate words to the helper.

1. Dispatch to `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" <args>`.
2. Print the helper's stdout to the user.
3. Stop. Do not enter Phase 0 or any pipeline phase.

## Feature request

{FEATURE_DESCRIPTION}

---


## Continuous loop mode (--loop)

When the operator invokes `/autospec --loop "<initial prompt>"` (alias
`--continuous`), this skill runs the full pipeline (Phases 1-6) once, then
harvests the Phase 6 final report (`.autospec/run-summary.md` written by
`autospec-write-run-summary.sh`) and re-iterates until one of six
termination conditions trips. The loop is driven by the shared library at
`lib/autospec-loop.sh` — the single source of truth shared with
`/autospec-refine --continue` and `/autospec-continue --loop`.

Flags:
- `--loop` (alias `--continuous`) — enable continuous-iteration mode.
- `--max-iterations N` — cap the loop at N iterations (default 5, env
  override `AUTOSPEC_LOOP_MAX_ITERATIONS`).
- `--skip-refine` — pipe the harvested next-prompt straight into the next
  /autospec invocation without first running `/autospec-refine` on it.

Termination conditions (priority order):
1. `evidence_based_stop` — the run-summary contains a `STOP: <reason>` marker.
2. `convergence_clean` — harvest returns empty / `(none — converged)`.
3. `oscillation_detected` — iteration N+1's harvested prompt hash equals
   iteration N's (same work proposed twice in a row).
4. `operator_stop` — `~/.autospec/stop.flag` or
   `~/.autospec/refine-loop-stop.flag` present at iteration boundary.
5. `budget_cap_reached` — `AUTOSPEC_LOOP_TOKEN_CAP` (default 2M tokens) or
   `AUTOSPEC_LOOP_TIME_CAP` (default 6h / 21600s) exceeded.
6. `round_cap_reached` — `--max-iterations` hit without any of the above.

Per-iteration loop:
1. Run the full /autospec pipeline (Phase 1-6) on the current prompt.
2. Read `$REPO_ROOT/.autospec/run-summary.md` and pass it to
   `autospec_loop_harvest_next_prompt` from the shared library.
3. If harvest is empty → `convergence_clean`. If it returns
   `STOP::<reason>` → `evidence_based_stop`. Otherwise the harvested text
   becomes the next iteration's prompt.
4. Unless `--skip-refine` is set, run the harvested prompt through
   `/autospec-refine` (4 lenses) before the next /autospec invocation.
5. Hash the harvested prompt; oscillation trips when the hash matches
   the previous iteration's.

Output artifacts:
- `.autospec/loop-summary.md` — markdown table with one row per iteration
  + final status + iterations executed.
- `<artifact-dir>/<slug>-loop.json` — per-iteration JSON record validated
  against `schemas/autospec-refinement-loop.schema.json`.

Safety guardrails inherit from the existing autonomy gate
(`autospec-autonomy-gate.sh`) — destructive remote actions still
surface for confirmation per iteration. The rate limit + injection guard
from `autospec-continue.sh` (PR #704) also apply.

Default behavior (no `--loop`) is unchanged: a single end-to-end pipeline
run with no harvest/re-iteration.

## Existing spec mode

Use this mode when the request asks to split, materialize, roadmap, decompose,
or turn an existing spec into GitHub issues. Examples:
`split existing spec`, `split latest spec`, `turn this spec into GitHub issues`,
`roadmap docs/specs/2026-05-01-example-design.md`, or
`materialize the newest spec`.

Do not run Phase 1 or Phase 2 in this mode. The design already exists; this
mode is a shortcut into Phase 3 using the same issue decomposition and Phase 3.5
classification path as the normal pipeline.

1. **Verify repo.** Run the Phase 0 repo probes. If the current directory is
   not a git repo with a GitHub remote, use Phase 0 bootstrap before selecting
   a spec.
2. **Resolve candidate specs.**
   - If the request contains an explicit `docs/specs/*.md` path, use that file.
   - Otherwise list tracked and untracked local files matching `docs/specs/*.md`.
   - Sort by ISO date in the filename (`YYYY-MM-DD-...`) descending. For files
     without a date prefix, sort after dated files by filesystem modified time.
   - The first item is the default "newest" spec.
3. **Ask on ambiguity.**
   - If zero specs exist, stop with: `No docs/specs/*.md files found. Create or
     point me at a spec before running existing spec mode.`
   - If exactly one spec exists and no explicit path was provided, use it.
   - If more than one spec exists and no explicit path was provided, ask the user
     to confirm the default newest spec or choose another path. Show at most the
     five newest candidates with relative paths. Do not file issues until the
     user answers.
4. **Verify selected spec is filed.**
   - The selected path must be under `docs/specs/` and end in `.md`.
   - The selected file must be tracked on `origin/main` before Phase 3, so child
     issues can cite a stable GitHub URL. Verify with:
     `git fetch origin` and `git cat-file -e origin/main:<spec-path>`.
   - If the selected file is missing from `origin/main`, stop and tell the user:
     `Selected spec is not on origin/main yet: <spec-path>. Land the spec first,
     or run normal /autospec so Phase 2 can create and merge the spec PR.`
5. **Continue at Phase 3.** Capture `{selected_spec_path}` and its GitHub URL as
   `https://github.com/{repo}/blob/main/{selected_spec_path}`. Run Phase 3 and
   Phase 3.5 using that selected spec. Then proceed to the existing Phase 3
   pre-impl gate.

<!-- autospec-block:harness-adapter -->

### Managed Project registration after Phase 0 bootstrap

When Phase 0 creates a GitHub repository, register it only after the required
remote read-back succeeds. Set `REPO` to the exact `owner/name` slug and set
`SPAWNED_FROM` to the source-spec URL when one already exists, otherwise to the
current Autospec run identity (or `bootstrap:<owner/name>` when neither exists):

```bash
gh repo view <owner>/<name> --json url,defaultBranchRef
REPO="<owner>/<name>"
SPAWNED_FROM="${SOURCE_SPEC_URL:-${AUTOSPEC_RUN_ID:-bootstrap:$REPO}}"
REGISTRATION_JSON=$("${AUTOSPEC_BIN:-autospec}" project onboard --repo-dir "$PWD" --repo "$REPO" --spawned-from "$SPAWNED_FROM") || {
  printf '%s\n' 'ERROR: managed Project repository registration failed before durable admission' >&2
  exit 1
}
case "$(printf '%s' "$REGISTRATION_JSON" | jq -r '.outcome // empty')" in
  reconciled) ;;
  journaled_projection_pending)
    printf '%s\n' 'WARNING: managed Project repository projection is durably journaled and pending' >&2 ;;
  *) printf '%s\n' 'ERROR: managed Project repository registration returned no supported typed outcome' >&2; exit 1 ;;
esac
```

Never run registration when remote verification fails. A typed pending outcome
does not roll back, delete, or recreate the verified repository; the managed
Project journal retains the pending projection for a later `project sync`.
Propagate every hard registration failure.

## Physical STL / CAD design guardrails

When the request involves STL, CAD, 3D-printable fixtures, workholding,
vacuum, pneumatic, hydraulic, dust-collection, hose, fitting, gasket, or
other physical models, carry these requirements into Phase 2 specs, Phase 3
child issues, and Phase 4 verification:

- Treat physical function as acceptance criteria, not visual intent. Verify
  mountability, tool access, fitting clearance, gasket compression, storage fit,
  printer build volume, and expected operating orientation.
- Do not allow blocked ports, obstructed hoses/tubes, capped passages, hidden
  dead-end channels, or geometry that interferes with the model's purpose.
- Around every port, fitting, connector, tap bore, hose socket, screw head, and
  service tool path, require at least 5 mm of free working clearance in every
  direction unless the user explicitly specifies a tighter envelope.
- Around every gasket path, require at least 5 mm of continuous plastic margin
  outside the gasket. Gasket grooves must not expose an edge or be crossed by
  mounts, screws, ribs, channels, holders, or workpiece contact geometry.
- For NPT or other tapped ports, prove the tap and installed fitting can enter
  straight or along the specified angled axis without collisions. Ports and
  fitting bosses must be integrated into reinforced body geometry, not left as
  fragile stand-alone protrusions.
- For low-flow vacuum systems, do not add relief valves, vents, restriction
  orifices, small balancing holes, or intentional leaks unless the user
  explicitly asks for them and the spec records why the pump can tolerate them.
- For sealed or flowing parts, require gas/fluid/vacuum/dust-flow simulation or
  deterministic geometry checks that prove no leaks, no disconnected internal
  circuits, no blocked passages, and adequate inlet-to-outlet connectivity.
- For dust collection, preserve maximum practical duct cross-section and
  suction path continuity. No vacuum tubing, air port, dust passage, or hose
  connection may be narrowed or blocked by ribs, mounts, bosses, decorative
  geometry, or slicer support assumptions.
- For functional openings such as dust hood mouths, plenum intakes, vacuum
  channels, gasket windows, and hose throats, add deterministic projection or
  section keepout checks. Prove that reinforcements, sockets, bosses, ribs, and
  cutters do not intrude into the working opening from any operating angle.
- For every generated STL, require visual QA from at least 16 angles: right,
  left, front, back, top, bottom, top-right 45, top-left 45, top-front 45,
  top-back 45, front-right 45, front-left 45, back-right 45, back-left 45,
  bottom-right 45, and bottom-left 45. Renders must be nonblank, well-framed,
  and reviewed for obvious gaps, overlaps, obstructions, unsupported fragile
  features, and unexpected protrusions.
- Regenerate from a clean build directory before release, then verify the build
  directory contains every expected STL plus any PDFs, sections, renders, and
  acceptance artifacts named by the spec.

---

## Harness detection (run once at skill start, before Phase 0)

Detect your harness by checking available tools before any phase:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness (model unknown, quota/capacity failure, authorization failure, or tool call returns an error for that model), silently retry the same subagent dispatch with `TIER_A`. Preserve the parent context on retry; for Codex native subagents, fork/inherit the current conversation context and use the latest top GPT model instead of moving the work into the main session. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run. Every "Tier A" and "Tier B" reference below resolves to these harness-specific values.

## Project configuration preflight

Before Phase 0, look for `.autospec/autospec.yml` at the repository root.

If the file is missing and the current directory is already inside a git repo,
run `/autospec-sweep init` behavior first:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-sweep-wizard.sh" init
```

Accept the wizard defaults unless repo findings require a project-specific
answer. The generated `.autospec/autospec.yml` is tracked project source and
must be committed with the next autospec spec/config change. After it exists,
read it before deciding which autospec phases are enabled.

If the file is missing because Phase 0 must bootstrap a brand-new repo, create
it immediately after the initial repo scaffold and before Phase 1. Use all
steps enabled by default, strict isolation, `team_personality: auto`, and
continuous improvement enabled for docs, tests, and code.

## Phase 0-3.5 — Delegate to /autospec-define

The define half (bootstrap, investigate, design, decompose, classify) is
owned by `/autospec-define`. Do NOT run these phases inline in this skill —
the full procedure lives in that skill, and re-inlining it here is exactly
the duplication this router removes.

> **MUST** read `skills/autospec-define/SKILL.md` (section "## Phase 0 — Bootstrap repo") and invoke `/autospec-define` (via the `skill` tool) to run Phases 0-3.5.

**Gate override.** `/autospec-define` ends at its own pre-impl gate, whose
default is `defer`. When the work was entered through `/autospec` (this
skill), override that default to **`run`** — the user invoked the umbrella
end-to-end skill expecting shipping, not plan-only. Pass the override into
`/autospec-define` so its gate question highlights `run`.

After `/autospec-define` completes (spec merged, issues filed, gate
answered), continue to the Phase 3 pre-impl gate below.

## Phase 3 pre-impl gate

After Phase 3 decomposition completes (and Phase 3.5 review-and-label has
applied `ctx:*` / `reasoning:*` labels), end Phase 3 by asking the user
this verbatim question (no auto-launch — always ask explicitly, per spec
§3.3):

> `"Spec written, N issues filed. Start /autospec-run now, defer to your external daemon, or keep refining? [run / defer / refine]"`

Substitute `N` with the actual issue count from Phase 3. Default highlight
is **`run`** for `/autospec` (the umbrella end-to-end skill — the user
invoked `/autospec` expecting end-to-end shipping, so the gate defaults
to `run`).

- **`run`** (default for `/autospec`) — proceed to Phase 4 by delegating to
  `/autospec-run` (the run-half pointer below).
- **`defer`** — print
  `"Issues are ready. Your external monitor will pick them up. Exiting."`
  and stop. For `/autospec`, this means skipping Phase 4–6 entirely; the
  just-filed `auto-implement` queue persists on the GitHub side so an
  external daemon (or a later `/autospec-run` invocation) can pick it up.
- **`refine`** — re-enter Phase 2 from question 1 (Architecture).

No daemon auto-detection — always ask explicitly.

Once `run` is selected and `/autospec-run` starts, do not ask operator questions again for this run unless a true hard blocker requires explicit manual recovery.

## Phase 4-6 — Delegate to /autospec-run

The run half (implement, monitor, final report) is owned by
`/autospec-run`. Do NOT run these phases inline in this skill — the full
monitor pseudocode, recovery ladder, and end-of-run flow live in that
skill, and re-inlining them here is exactly the duplication this router
removes.

> **MUST** read `skills/autospec-run/SKILL.md` (section "## Phase 4 — Background autonomous monitor") and invoke `/autospec-run` (via the `skill` tool) to run Phases 4-6.

`/autospec-run` handles the background autonomous monitor, periodic status
updates, and the Phase 6 final report (including the `.autospec/run-summary.md`
harvest contract that `--loop` mode consumes). It ends when the queue is
drained (`ALL_DONE`) or the operator stops the run.

---

## Constraints (apply throughout)

- **Cadence**: sub-hour polling needs an in-session background subagent or a local cron. Cloud cron services (e.g. Anthropic remote routines) typically have a 1-hour minimum and are not appropriate.
- **TDD non-negotiable** per AGENTS.md. Real services in tests, no DB mocks.
- **Branch-per-issue**, conventional commits, no force-push, no hook bypass.
- **Auto-merge** on success per AGENTS.md. Do not ask.
- **Context budget**: stay under 60%. Delegate everything possible.
- **Failure isolation**: a failed issue restores its `auto-implement` label so the next monitor cycle picks it up; it does not block the cascade unless dependent issues are downstream.
- **Fresh repo**: if Phase 0 created the repo, the very first commit is the scaffold (incl. `AGENTS.md`); the spec lands as the second commit; child branches branch off that `main`.
- **Small-LLM target**: child issues are sized and pre-staged for 32B-class local LLMs (e.g. qwen3-32B / Qwen3-30B-A3B on Ollama, 48 GB-class Macs). Bias toward smaller issues, sectional spec anchors, pre-staged file pointers, checkbox AC, and one Primary smoke test.

## Autonomous mode

`/autospec --autonomous "<request>"` skips all user-facing confirmation gates
in this end-to-end pipeline. Spec is generated directly from conversation
context + repo evidence; pre-implementation gate defaults to `run`; issue
creation runs without approval.

Triggers (any one enables autonomous mode):

1. **Explicit flag** — `/autospec --autonomous "<request>"`.
2. **Operator preference** — `~/.autospec/autonomous.flag` exists; every
   `/autospec` invocation defaults to autonomous.
3. **autospec-listen phrase detection** — the deterministic listener-match
   classifier recognizes "fix X automatically", "no confirmation",
   "just do it", "non-interactive", "go autonomous" and routes with
   `--autonomous`.

Gates bypassed in autonomous mode:

- **Phase 2 brainstorm** — clarifying questions skipped. Spec is drafted
  inline from (a) the user's request text, (b) the last ~10 turns,
  (c) `git log --since="7 days ago"` for recent change context, and
  (d) `docs/specs/**` for similar prior work.
- **Phase 3 pre-implementation gate** — defaults to `run`.
- **Issue body confirmation** — `gh issue create` runs without approval.

Autonomous mode honors `--autonomous` end-to-end so Phase 4–6 telemetry can
distinguish autonomous runs from interactive ones.

### Autonomous spec drafting

When `--autonomous` is set, do NOT ask the user. Synthesize the spec from
conversation + repo evidence. For each unverifiable inference, mark the line
in the spec body with a `> AUTONOMOUS ASSUMPTION:` blockquote so the spec PR
review can catch wrong inferences. If a critical field cannot be inferred,
pick the conservative default and note it under an **Autonomous assumptions**
section in the spec.

If the orchestrator cannot generate a coherent spec from available context
(e.g. the request is too vague — "make it better"), autonomous mode FAILS
LOUDLY with `code_health:autonomous_input_insufficient` rather than silently
picking arbitrary defaults. The operator gets the specific missing fields
and a recommended interactive command to retry.

### Safety guardrails (autonomous)

`autospec-autonomy-gate.sh` is called before each "would-have-asked"
decision point. The following are NEVER bypassed:

1. **Destructive remote actions** — prod DB writes, force-push to protected
   branches, mass label changes, repo deletion, `gh repo archive`,
   `gh release delete`, `git push --force` always surface for confirmation.
   Gate exit 1 = "ask anyway".
2. **Out-of-scope file changes** — if Phase 4 implementer's planned file
   list extends outside the auto-detected scope (Goal + Implementation
   outline files), the gate surfaces a confirmation listing the unexpected
   files.
3. **Cost gate** — if estimated total tokens exceed
   `AUTOSPEC_AUTONOMOUS_TOKEN_CAP` (default 500k), surface a
   one-time go/no-go even in autonomous mode.
4. **Existing rules in `feedback_autospec_autonomy_scope.md` remain in
   force.**

Invoke the gate before each gate-trigger point:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-autonomy-gate.sh" \
  --check all \
  --tokens "$TOKEN_ESTIMATE" \
  --files "$PLANNED_FILES" --scope "$SCOPE_PREFIXES" \
  --intent "$USER_REQUEST"
```

Exit 0 = autonomous OK, proceed. Exit 1 = surface confirmation. Exit 2 =
invocation error.

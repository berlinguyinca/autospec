
# autospec-listen (harness-neutral)

Conversation listener that watches for issue-filing or spec-writing intent
mid-conversation and dispatches to the right downstream flow without forcing
the user to remember a slash command.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-listen -->

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal trigger flow:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-listen/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-listen.md`
   - Codex CLI:   `~/.codex/prompts/autospec-listen.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not enter any trigger flow. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-listen found; run install.sh first.` and exit.

## Harness detection (run once at skill start)

Detect your harness by checking available tools before dispatching work:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness (model unknown, quota/capacity failure, authorization failure, or tool call returns an error for that model), silently retry the same dispatch with `TIER_A`. Preserve the parent context on retry; for Codex native subagents, fork/inherit the current conversation context and use the latest top GPT model instead of moving the work into the main session. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run.

## Required capabilities & harness adapter

This skill assumes four capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read recent conversation    | harness-native message buffer        | harness-native message buffer            | harness-native message buffer            | Ask the user to paste the relevant turns           |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Run `gh` CLI                | shell tool                           | shell tool                               | shell tool                               | Have the user run the command and confirm          |
| Hand off to another skill   | `Skill` invocation (`/autospec-define`) | nested skill invocation                | `/autospec-define` slash command         | Print the suggested command for the user to run    |
| Subagent model tier         | Tier B: `sonnet` + medium thinking; Tier A only if escalating to /autospec-define which honors its own tier policy | Tier B: smaller-tier `task` + medium reasoning; escalation honors target skill | Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium`; escalation honors target | Honor AGENTS.md tier mapping; retry the same subagent UP on unavailability |
<!-- autospec-block:harness-adapter-core -->

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. Per AGENTS.md, this listener runs at **Tier B** (it's a router, not a designer); when it hands off to `/autospec-define` for a spec trigger, the downstream skill applies its own tier policy. Fall back UP the tier on quota/capacity or other unavailability by retrying the same subagent with the stronger tier while preserving parent context.

## Trigger flow

> **Model tier:** Tier B (implementation work) — this listener is a router; subagent dispatches inside its trigger flows (drafting issue bodies, summarizing recent conversation) use cheaper models per AGENTS.md. Spec-trigger handoff to `/autospec-define` lets the downstream skill apply its own tier policy.

## Keyword auto-routing

Beyond the explicit issue/spec triggers, this listener routes common build/change verbs into the matching autospec skill so the pipeline is the default path. Routing is gated by an imperative-intent check and always offers a one-line opt-out.

**Intent gate.** Do not classify by eye. Delegate to the deterministic classifier, passing the user's latest message verbatim:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/listener-match.sh" --classify "<user message>"
```

It emits `{"match":bool,"skill":...,"trigger":...,"intent":"imperative|incidental|none","confidence":0-1,"autonomous":bool,"chain":<skill|null>,"gate":<string|null>}`. The gate is biased to false-negatives: descriptive ("the design is nice"), past-tense ("I reviewed it"), negated ("don't implement yet"), and interrogative ("should we redesign?") uses return `match:false` and MUST NOT route. Prefer no route over a misfire.

The two new trailing fields:
- `chain` — after the primary skill completes, invoke the chained skill (e.g. `/autospec-run` after `/autospec-refine`). Null when no chain applies.
- `gate` — before routing, the listener must satisfy the named context gate; `auto-implement-open` means run `gh issue list --repo <repo> --label auto-implement --state open --json number --jq 'length'` and route only if > 0, else stay in plain mode. `explore-confirm` means the route launches `/autospec-explore`, which starts a perpetual autonomous research + ship loop on an isolated sandbox branch whose PRs target the sandbox branch, never main; before invoking, print exactly that consequence and require ONE explicit confirmation, routing only on an affirmative reply. Null means no gate.

**Verb → skill map (D3).**

| Verb(s) in an imperative request | Routes to |
|----------------------------------|-----------|
| `design` / `new feature` / `spec` | `/autospec-define` |
| `implement` / `build` / `ship` | `/autospec-run` (or `/autospec` end-to-end when no issues exist yet) |
| `refine` / `optimize` / `polish` / `improve` / `tune` | `/autospec-refine` |
| `run` (scoped: "run it", "run the loop", "run autospec"; bare "run" only when open `auto-implement` issues exist) | `/autospec-run` |
| `refine and run` / `tune up` (combined) | `/autospec-refine` then `/autospec-run` |
| `review` | `/autospec-review` |
| `explore` / `discover` (only with an explicit build/ship/improve action connector, e.g. "explore and ship", "discover features to build") | `/autospec-explore` (gate `explore-confirm`) |
| `fix` (imperative, e.g. "fix the login flow"; trivial-fix phrasings like "fix the typo", "quick fix", "fix the formatting" stay plain) | the umbrella `/autospec` (no gate) |
| `autospec …` | the umbrella `/autospec` |
| GitHub Projects board URL (`https://github.com/(orgs\|users)/<name>/projects/<n>`, optional `/views/<n>`) **with** a build/ship verb (`ship` / `build` / `implement`) | `/autospec-project ship <url>` (no gate) |
| GitHub Projects board URL alone, **no** build/ship verb | `/autospec-project <url>` (bare resolve — zero mutation, no gate) |

**GitHub Projects board routing (Task 12).** A message that contains a GitHub
Projects v2 board URL — `https://github.com/orgs/<org>/projects/<n>` or
`https://github.com/users/<user>/projects/<n>`, optionally with a trailing
`/views/<n>` — carries board intent, checked before the generic
implement/build/ship and `autospec …` rows above so it wins over a bare
`autospec` or `ship` word in the same message. The classifier reports this as
skill `autospec-project` with trigger `project-ship` or `project-resolve`;
extract the URL from the user's message with the same regex and build the
command yourself, since the classifier does not echo the URL back:

- **URL + ship verb** (e.g. "autospec ship this project for me:
  `https://github.com/orgs/InferWeave/projects/2`") → invoke
  `/autospec-project ship <url>`. `ship` starts autonomous multi-repo work,
  so this requires the verb explicitly present — a bare URL never implies it.
- **URL alone**, no ship verb (e.g. a pasted board link, or "what's on
  `<url>`") → invoke `/autospec-project <url>` (bare mode: resolves and
  prints the plan, performs zero mutation). Reading a board is not a
  commitment to drain it — this asymmetry is deliberate.

Both rows still honor the intent gate (biased to false-negatives) and the
one-line opt-out below.

**Auto-route behavior.** When the classifier returns `match:true` with `intent:imperative`, print exactly one line then invoke the mapped skill via the harness skill-invocation primitive:

```
Routing to /<skill> — say "plain" to opt out.
```

If `gate` is non-null, perform the gate check first and skip routing (plain mode) if unmet. For `gate:explore-confirm` (skill `autospec-explore`), do NOT auto-route on the one-line opt-out alone: first print exactly that `/autospec-explore` starts a perpetual autonomous research + ship loop on an isolated sandbox branch whose PRs target the sandbox branch, never main, then require ONE explicit confirmation and invoke `/autospec-explore` only on an affirmative reply; the existing `plain`/`no` escape still cancels. If `chain` is non-null, after the mapped skill completes, invoke the chained skill (printing one opt-out line as usual).

On a non-match or `intent:incidental`/`none`, do nothing — the session proceeds normally in plain mode.

**Post-approval execution-ready routing.** After an autospec/spec/refine flow writes
an approved spec or implementation plan, do not ask the user to choose between
subagent-driven and inline execution. Record the recent workflow state for the
classifier using the narrow env hints below, then pass the user's approval turn
verbatim:

- `AUTOSPEC_LISTENER_APPROVED_SPEC_PATH` — path to the approved spec/design.
- `AUTOSPEC_LISTENER_PLAN_PATH` — path to the written implementation plan.
- `AUTOSPEC_LISTENER_SOURCE_SPEC_PATH` — source spec path, if the plan was derived from one.
- `AUTOSPEC_LISTENER_REPO` / `AUTOSPEC_LISTENER_BRANCH` / `AUTOSPEC_LISTENER_BASE` — minimal repo routing metadata.
- `AUTOSPEC_LISTENER_PLAN_TS` — timestamp for the saved plan or handoff artifact.
- `AUTOSPEC_LISTENER_AUTOSPEC_REQUESTED=1` — the thread previously requested autospec routing.
- `AUTOSPEC_LISTENER_AUTONOMOUS_REQUESTED=1` — the thread previously requested autonomous routing.
- `AUTOSPEC_LISTENER_AUTO_IMPLEMENT_OPEN=<count>` — open `auto-implement` issue count, from `gh issue list --repo <repo> --label auto-implement --state open --json number --jq 'length'`.

For approval phrases such as `looks good` or `approved`, the classifier emits
`trigger:"post_approval_execution_ready"`. If
`AUTOSPEC_LISTENER_AUTO_IMPLEMENT_OPEN` is greater than zero, route to
`/autospec-run`; otherwise route to `/autospec --autonomous`. Print exactly:

```
Routing to /autospec --autonomous — say "plain" to opt out.
```

or, when open `auto-implement` issues exist:

```
Routing to /autospec-run — say "plain" to opt out.
```

Explicit opt-outs (`plain`, `no`, `no workflow`, `just chat`, `do not implement`),
conceptual questions, destructive/production requests, and descriptive mentions
of autospec remain plain mode.

**Completed Plan-mode handoff routing.** When any harness exits Plan mode after
writing a concrete implementation plan, autospec is the default next step. Set
`AUTOSPEC_LISTENER_PLAN_EXIT_READY=1` plus
`AUTOSPEC_LISTENER_PLAN_PATH=<path>` (or
`AUTOSPEC_LISTENER_PLAN_ARTIFACT=<summary>` when no path exists) and pass the
latest Plan-mode exit text or the user's approval turn through the classifier.
The classifier emits `trigger:"plan_exit_ready"` for the direct Plan-exit
handoff and `trigger:"plan_approved_ready"` for approval phrases such as
`continue`, `proceed`, `go`, or `ship it` after a completed plan.

When `AUTOSPEC_LISTENER_AUTO_IMPLEMENT_OPEN` is greater than zero, route to
`/autospec-run`; otherwise route to `/autospec --autonomous`. The JSON output
also carries `plan_path`, `source_spec_path`, `repo`, `branch`, `base`, and
`plan_ts` fields so the handoff can pass or summarize the saved plan instead of
restarting discovery from scratch. Suppress generic execution-choice prompts
such as "Subagent-Driven or Inline Execution?" when this routing state is
present; print only the one-line autospec opt-out:

```
Routing to /autospec --autonomous — say "plain" to opt out.
```

or:

```
Routing to /autospec-run — say "plain" to opt out.
```

Do not route when `AUTOSPEC_LISTENER_PLAN_EXIT_BLOCKED=1`,
`AUTOSPEC_LISTENER_PLAN_DESTRUCTIVE=1`, the plan text says it is blocked,
ambiguous, destructive, production/credential-gated, or still requires explicit
approval, or the user asks a conceptual question about the plan. The one-turn
escape includes `plain`, `no`, `no workflow`, `just chat`, `do not implement`,
`stop`, and `cancel`.

**Escape (D5).** If the user's next turn is the stop word (`plain` or `no`), cancel routing and proceed in plain mode for that request. The existing `file an issue` / `write a spec` triggers continue to fire their own flows below and are unaffected.

## Issue trigger flow

When the user utters a phrase from the **Issue triggers** list (see `references/trigger-keywords.md`), run this 7-step procedure:

1. **Read recent context.** Pull the last ~10 conversation turns from the harness's message buffer. If that capability is missing, ask the user to paste the relevant excerpt.
2. **Synthesize draft body.** Build a markdown body with three sections:
   - `## Goal` — exactly one sentence summarizing what the issue should achieve.
   - `## Context` — relevant chat excerpts that motivate the issue (verbatim quotes, ≤6 lines).
   - `## Suggested AC` — ≤3 bulleted acceptance-criteria checkboxes (`- [ ]` style).
   The full body MUST be ≤400 words.
3. **Print the draft inline** so the user can read it before approving.
4. **Confirm via the harness primitive.** Ask exactly: `File this as a new issue? [yes / edit / cancel]`. Use `AskUserQuestion` on Claude Code, an inline prompt elsewhere; if no question primitive is available, ask in the response and wait for the next turn.
5. **On `yes`**, infer the title and run:
   ```bash
   issue_url="$(gh issue create --title "<inferred-title>" --body "<draft-body>" --label needs-classify)"
   bash "${AUTOSPEC_SCRIPTS_DIR:-$PWD/skills/autospec-shared/scripts}/project-sync-issue.sh" "$issue_url" "$PWD"
   ```
   Then post the resulting issue URL back to the chat. Project sync is best-effort but journals the issue before remote reconciliation, so a failed projection remains retryable. The `needs-classify` label keeps the issue out of the auto-implement queue until `/autospec-classify` sizes it.
6. **On `edit`**, accept free-text revisions from the user, re-render the draft, and re-confirm (loop back to step 4).
7. **On `cancel`**, print `OK, cancelled.` and silently exit. No persistence, no learning, no suppression list.

### Title-inference rule

Build the title from the Goal sentence using the following deterministic rule:

1. Take the `## Goal` sentence verbatim.
2. **Strip leading verbs** `Implement` / `Add` / `Fix` (case-insensitive, with any trailing whitespace) from the front of the sentence.
3. **Truncate** the remainder to 80 characters total (after the prefix in step 4 is added).
4. **Prefix with a conventional-commit tag** inferred from the original leading verb:
   - `Add` / `Implement` / new behavior → `feat: `
   - `Fix` / bug language → `fix: `
   - documentation-only language → `docs: `
   - refactoring language → `refactor: `
   If inference is ambiguous, leave the tag off entirely and let `/autospec-classify` add one later.

Example: Goal `Implement a /loop slash command that re-runs the last skill` → title `feat: a /loop slash command that re-runs the last skill`.

## Spec trigger flow

When the user utters a phrase from the **Spec triggers** list (see `references/trigger-keywords.md`), this listener acts as a **router only** — it does NOT duplicate the Phase 2 brainstorming logic that lives in `/autospec-define`.

1. **Confirm.** Ask exactly: `Start the autospec design Q&A on <inferred-topic>? [yes / cancel]`. Use `AskUserQuestion` on Claude Code, an inline prompt elsewhere; if no question primitive is available, ask in the response and wait for the next turn. The `<inferred-topic>` is a short noun phrase synthesized from the trigger phrase and the most recent user turn (≤10 words).
2. **On `yes`**, hand off to `/autospec-define` via the harness's skill-invocation primitive (Claude Code: `Skill` tool; OpenCode: nested skill invocation; Codex CLI: emit the `/autospec-define` slash command). The downstream skill applies its own tier policy. Do NOT inline any Phase 2 questions here.
3. **On `cancel`**, print `OK, cancelled.` and silently exit. No persistence, no learning, no suppression list.

## Autonomous mode

The deterministic listener-match classifier recognizes autonomous-intent
phrasing and routes the request with `--autonomous` so downstream
`/autospec`, `/autospec-define`, and `/autospec-run` skip user-facing
confirmation gates.

Trigger phrases (case-insensitive, classified imperative):

- "fix X automatically"
- "no confirmation"
- "just do it"
- "non-interactive"
- "go autonomous"
- "autonomous mode"
- "run autonomously"

On a `match:true` + `intent:imperative` classification where the message
contains one of these phrases, the listener routes to the mapped autospec
skill with `--autonomous` appended:

```
Routing to /autospec --autonomous — say "plain" to opt out.
```

If the classifier returns `intent:incidental` or `none`, do NOT route — the
session continues in plain mode. The same biased-to-false-negative rule
applies: descriptive, past-tense, negated, and interrogative uses MUST NOT
route.

The autonomous routing inherits the listener's existing `plain`/`no` escape:
the user's next-turn stop word cancels routing and proceeds in plain mode
for that request.

---
name: feedback_refine_then_run_workflow
description: "User's standard autospec workflow is /autospec-refine then /autospec-run; prefers keyword auto-routing shorthands over typing slash commands."
metadata:
  node_type: memory
  type: feedback
  originSessionId: f656413f-97c2-491e-a407-119d14e2bcbe
---

The user nearly always runs `/autospec-refine` on a prompt/feature request and *then* `/autospec-run`, keeping a review checkpoint between the refine and the implementation drain. They asked for keyword auto-routing so they don't have to type the slash commands.

Decided routing (added to `autospec-listen`'s verb→skill map, issue #852):
- `refine` / `optimize` / `polish` / `improve` / `tune` → `/autospec-refine`
- `run` → `/autospec-run`, but **context-gated**: scoped phrases (`run it`, `run the loop`, `run autospec`, `drain the queue`) route unconditionally; **bare `run` routes only when open `auto-implement` issues exist**, and `run the tests/build/server/lint/...` never route.
- Combined shorthand (`refine and run`, `tune up`, `optimize and run`) → `/autospec-refine` then `/autospec-run`.

**Why:** This is the user's habitual pipeline; the context gate on bare `run` was their explicit choice to avoid misfiring on everyday "run the tests" phrasing.

**How to apply:** When the user says these verbs imperatively, prefer routing through the autospec pipeline. Respect the intent gate (questions/negation/past-tense never route). The classifier (`scripts/listener-match.sh`) is the source of truth and must stay byte-identical across the lockstep trio (SKILL.md + codex + opencode). Related: [[project_turbo_integration_design]].

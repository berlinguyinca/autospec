---
name: project_babysit_tax_autonomy_charter
description: "Session-mining shows the operator's confirmation turns are ~always rubber-stamps of the agent's own stated recommendation; autospec needs a standing Autonomy Charter"
metadata: 
  node_type: memory
  type: project
  originSessionId: 5bdc71c2-d23b-4462-9d28-61ca3ab6e3dd
---

Mined all ~2963 Claude + ~1538 Codex transcripts (2026-06-25). Pure low-information steering turns: Claude 5.2% of human turns, Codex 13.4%; autospec project = 76 turns (22 bare "yes/looks good"). Dominant bucket is `approve_long` ("yes, do X" / "ok fix that one once and for all" / "review the whole project for gaps").

**Why:** Every gate is the same shape — the agent *states its own recommendation*, then asks permission, and the operator always grants it. Four recurring stop-points:
1. **Design ratification** before spec-writing ("State machine make sense? Test plan look sound? UX feel right?") — operator never overrides.
2. **Spec→plan→implement handoff gates** built into autospec-define ("review the spec before I invoke writing-plans").
3. **"Want me to commit / file these gap issues / run fixture-gen?"** — permission for the obviously-next reversible action.
4. **End-of-queue stall** ("queue empty, next work: none, standing by") — operator always replies "review the whole project for gaps," which IS [[feedback_autospec_design_prefs]]'s explore/review loop.

Plus a `poll` bucket during async CI/monitor waits ("how do they look?") = the agent goes silent instead of pushing transition notifications.

**How to apply:** Build a single shared **Autonomy Charter** referenced by all autospec entrypoints (no central one exists today; gates are scattered across autospec-define/run/the orchestrator). Rule: *recommendation = action*. Auto-proceed on reversible/local steps and on the agent's own stated next step; pause ONLY for destructive remote ops, irreversible deletes, genuine no-clear-winner forks, or cost thresholds — consistent with [[feedback_autospec_autonomy_scope]]. Collapse design-ratification into "Decisions I made (flag if wrong)" + self-review. Auto-advance define→run; on queue drain auto-chain into /autospec-explore or /autospec-review. Use PushNotification on async-wait transitions instead of waiting to be pinged. Related: [[feedback_proactively_query_memory]].

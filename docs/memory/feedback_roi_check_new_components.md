---
name: feedback-roi-check-new-components
description: User demands concrete ROI justification for each new skill/component/abstraction; defaults to invoking upstream over forking
metadata: 
  node_type: memory
  type: feedback
  wing: synthesis
  drawer_class: lesson
  originSessionId: 0a77c1fd-c243-4bf9-b3fb-4f83ae5f9830
---

When proposing to add skills, forks, abstractions, or metadata schemas, every component must justify itself with a concrete, named win — not "completeness," "symmetry," or "future-proofing." Components that don't earn their weight get cut.

Exact quote that triggered the rule (2026-05-17, turbo↔autospec integration brainstorm):

> "sounds all good, make sure that all these things actually improve the code and are not just burning tokens here"

This came after I'd designed an 11-skill fork. Honest re-audit cut it to 5: only fork the skills that run *autonomously inside subagents on small LLMs* (where tier-aware prompts and autospec guardrails matter). For operator-interactive skills run on Opus, invoke upstream directly — forking adds maintenance burden for zero gain.

**Why:** [[feedback_autospec_design_prefs]] already establishes correctness>>speed and conservative guardrails, but this is a different axis — about *design surface area*. Adding ceremony costs maintenance, drift, and prompt token bloat across every invocation. Pair this with [[feedback_autospec_skill_per_capability]]: skill-per-capability is correct when each skill has a *distinct user-facing capability*, not when forking just to brand-stamp upstream behavior.

**How to apply:**
- For each proposed new skill/fork/metadata field, name the consumer that benefits *today*. If the consumer doesn't exist yet, defer the addition until it does.
- Default to "invoke upstream" over "fork upstream." Only fork when: (a) runs autonomously inside a subagent on a small LLM tier, (b) needs autospec-specific guardrails baked in, or (c) needs integration glue (auto-merge, lock-step labels) that upstream doesn't know about.
- When user says "sounds good" to a design, do not interpret as approval to ship — pressure-test scope first. They will pushback if anything is ceremony.
- Schema additions (e.g., shell YAML in issue body) only land alongside a consumer change. Otherwise they're decoration the operator has to maintain.

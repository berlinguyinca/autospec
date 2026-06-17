---
name: feedback_lint_body_too_long_counts_injected_metadata
description: "lint-issue.sh BODY_TOO_LONG counts auto-injected Model-fit + Shared-contracts blocks, so every classified child trips needs-quality-bar even when the authored body is in budget"
metadata: 
  node_type: memory
  type: project
  originSessionId: 065dc9be-3716-42b0-9667-ccb4ac86d20e
---

In an autospec-define run (harmonize spec, issues #1137-1146), ALL 10 children
got `needs-quality-bar` with the single finding `BODY_TOO_LONG` (457-514 words).
Root cause: Phase 3 trims each child body to ≤400 words, then Phase 3.5 appends
the `## Model fit` block and Phase 3.75 appends the `## Shared contracts` block
(~104 words combined). `scripts/lint-issue.sh`'s word count includes those
auto-injected metadata blocks, so the post-filing audit re-flags every child
even though the implementer-load-bearing prose is within budget.

**Why:** the 400-word cap exists to keep a 32B-class implementer's context small,
but the injected blocks are autospec's own metadata, not authored implementer
load — counting them defeats the cap's intent and makes `needs-quality-bar`
noise on every decomposition.

**How to apply:** treat a blanket `needs-quality-bar` whose ONLY finding is
`BODY_TOO_LONG` as benign when the overflow equals the injected metadata; don't
block the gate on it. Real fix (future self-improvement): have `lint-issue.sh`
BODY_TOO_LONG exclude content between `<!-- autospec-classify:* -->` and
`<!-- autospec-shared-contracts:* -->` markers from the word count, OR run the
post-filing audit on the pre-injection body. Relates to
[[feedback_decompose_trio_prose_goldens_atomic]].

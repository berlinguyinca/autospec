---
name: feedback_decompose_trio_prose_goldens_atomic
description: "Never decompose a spec so \"trio prose edit\" and \"golden regen\" land in separate issues — validate fails closed on the prose-only intermediate; keep them one atomic issue"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 59916493-7c02-4311-889a-2cbeccc1c071
---

When `/autospec-define` decomposes a spec, the ≤3-files-touched cap tempts it to
split a skill change into "edit the trio prose" (issue X) and "regenerate the
sha256 goldens" (issue X+1). This is WRONG: `autospec validate`'s
block-expansion golden gate + `check_lockstep` require the trio prose change and
its regenerated goldens in the SAME commit, so the prose-only issue can never
merge green on its own. The same applies to a trio whose new prose is gated by a
NEW `check_*_contract` added in a later issue.

**Why:** this bit three times in the 2026-06-16 multi-spec run — #1067/#1068
(autospec-run), #1084/#1085 (autospec-explore), #1100/#1101 (define+split). Each
time I had to recombine the split issues into one implementer that did prose +
goldens (+ any new validate gate) atomically and closed both issue numbers.

**How to apply:**
1. At decompose time, keep "trio prose + its goldens (+ the validate gate that
   asserts the new prose)" as ONE issue even if it exceeds 3 files — correctness
   beats the file cap. Tell the decomposer this explicitly.
2. If they're already split in the queue, dispatch ONE implementer for the pair,
   instruct it to do prose + golden regen in one commit/PR, and `Closes #X` +
   `Closes #X+1`.
3. The drainer must never run the prose-only issue alone expecting green.

Related: [[feedback_skill_golden_derivation_workflow]],
[[feedback_validate_sh_lockstep_checks]], [[feedback_per_session_worktree_isolation]].

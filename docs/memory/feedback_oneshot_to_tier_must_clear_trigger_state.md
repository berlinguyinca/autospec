---
name: feedback_oneshot_to_tier_must_clear_trigger_state
description: folding a one-shot skill into a per-cycle conductor tier requires the skill to CLEAR the state its trigger signal reads, or the tier re-fires every cycle and starves lower-priority tiers
metadata:
  node_type: memory
  type: feedback
  originSessionId: d87fae41-0795-45c0-9afe-909bd9bc37fb
---

When you convert a **one-shot skill** (run once, hand off) into a **per-cycle
conductor tier** (evaluated every loop iteration), the skill must actively
**clear/transition the very state its tier's trigger signal reads** after it
services that work — otherwise the trigger stays hot forever.

**Why:** a one-shot skill never had to clean up its trigger; nothing re-read
it. A conductor tier's selection cascade re-evaluates the signal each cycle. If
the signal is "count of open issues carrying label X" and the skill services
the work by filing a *new* artifact (a control issue, a ledger line) without
removing label X from the *source* issue, the count never drops → the tier
fires again next cycle → it re-does the same work (duplicate control issues,
duplicate notifications). Worse, if that tier out-ranks other tiers in the
cascade (lower emit-number = evaluated first), the perpetually-hot tier
**starves** every tier below it — the conductor never advances to define/measure.

Bit the growth fold-in (Plan 5, PR #1725): Tier G2 counted open
`growth/needs-draft` issues, but grow-run R2/R3 queued/serviced work without
stripping that label from the source issue. Opus whole-branch review caught it.
Fix has TWO mirrored halves — the producer side (R2 retires the source issue:
strip `needs-draft`, add `queued`/`rejected` on terminal outcomes; leave it only
for an intentional retry like a cadence window) AND the consumer side (R3 clears
the decision label on every serviced branch so the shared pending count falls to
zero). Also wire EVERY trigger the spec lists (G2 fires on drafts-to-draft OR
approval-decisions — the approval poll was missing, so R3 only ran by accident of
the re-fire bug).

**How to apply:** when decomposing/authoring a capability as a conductor tier,
for each trigger signal ask: "after the tier does its work, what makes this
signal go back to not-firing?" If the answer isn't an explicit
label-strip/close/state-transition the skill performs, you have a re-fire +
starvation bug. Bound the diagnostic too (once-per-run, not per-cycle, in a
perpetual loop). Related:
[[feedback_capabilities_are_conductor_tiers_not_new_conductors]],
[[feedback_monitor_silent_exit]].

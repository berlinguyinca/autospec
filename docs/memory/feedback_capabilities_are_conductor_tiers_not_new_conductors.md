---
name: feedback_capabilities_are_conductor_tiers_not_new_conductors
description: "new autonomous capabilities fold into autospec-autonomous as capability-gated tiers, never a parallel conductor; a second conductor duplicates control-channel/park/resilience and splits the WSJF quota"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: d87fae41-0795-45c0-9afe-909bd9bc37fb
---

When adding a new perpetual/autonomous capability (growth, or any future "run
this forever" workstream), DO NOT build a second conductor skill. Fold it into
`autospec-autonomous` as a **capability-gated tier**, following the shipped F4–F8
extension model.

**Why:** the project's own doctrine (autospec-autonomous SKILL.md: "This skill is
a conductor, not a new engine. It reuses without reimplementing…"; SKILLS.md:
"Use the narrowest skill that matches the job"). A parallel conductor duplicates
the Tier-0 control channel, premerge/main-health gates, resilience/locking,
digest/notify, and quota machinery — and two conductors compete for one
operator's quota, defeating the single WSJF-ranked waterfall. The surface is
already ~34 skills with a history of skills operators "never saw."

**The extension recipe (every existing tier follows it):**
1. New `{"tier":N,"action":"..."}` branch in `scripts/autonomous-waterfall.sh`
   (append after Tier 4 to avoid renumbering tier-number test contracts).
2. New `elif [ "$_action" = "..." ]` branch in `scripts/lib/autospec-loop.sh`
   with an `AUTOSPEC_<CAP>_CMD` env seam → real script → graceful-fallback chain
   (Tier 3 architecture branch ~lines 1290-1333 is the copy template), a
   `_tierN_dry_cycles` counter, and `_work_done`.
3. A standalone `autonomous-<cap>.sh` doing the work, self-gated on staleness.
4. Capability-detection gate: inert unless the repo has the opt-in
   `.autospec/<cap>.yml` (growth → `.autospec/growth.yml`), same as F8 web/RAG.
5. Score candidates through the shared `autonomous-prioritize.sh` WSJF ranker so
   the new work competes fairly under ONE quota — do NOT give it a separate
   spend budget (that re-creates the two-loop problem).

**Historical evidence (resolved):** the 2026-07-10 growth rollout applied this
pattern as conductor tiers 5–7 while keeping grow-define/grow-run as reusable
halves. Its initial `growth:artifact` quality-gate bypass was subsequently fixed
in the shared `fab-route.sh` label router. Preserve the architectural lesson;
do not treat that shipped migration as pending work. Related:
[[feedback_roi_check_new_components]], [[feedback_autospec_skill_per_capability]].

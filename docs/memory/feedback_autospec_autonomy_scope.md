---
name: /autospec autonomy scope — auto-merge spec PRs, don't ask
description: User expects /autospec to admin-merge spec PRs and routine workflow plumbing without asking; major gates like run/defer/refine are still OK to surface
type: feedback
wing: synthesis
drawer_class: lesson
originSessionId: ff14de95-2db2-420a-9835-0401379148a3
---
When running `/autospec`, the user's standing expectation is that the
skill is **autonomous for routine workflow plumbing** — including
admin-merging the spec PR after Phase 2 — and only surfaces **major
branching decisions** (e.g. the Phase 3 `run / defer / refine` gate).

**Why:** verbatim from user during the 2026-05-01 startup-self-update
session, after I opened spec PR #111 and asked how to merge it:

> "you are autospec, you are supposed to do this automatically wihtout
> me being involded. Please add this to the feature. to automatically
> merge the spec pr's and not involve me at this point"

And later, when the harness hook re-denied the admin-merge:

> "you are allowed todo admin merges"

This was strong enough that the spec was amended in-flight (§3.6 of
`docs/specs/2026-05-01-autospec-startup-self-update-design.md`) to grow
two children (#125, #126) wiring spec-PR auto-merge into the
`autospec` and `autospec-define` Phase 2 trios, and one child (#124)
extending `AGENTS.md ## Auto-merge authority` to cover spec PRs whose
head matches `feat/spec-*` or whose body cites `docs/specs/`.

**How to apply:**

1. **Spec PR opening + merge** — after writing the design doc, open the
   PR and `gh pr merge <#> --admin --squash --delete-branch` directly.
   Do NOT use `AskUserQuestion` to ask about merging. The pattern is
   "open + admin-merge in one breath", same as Phase 4 does for
   `auto-implement` PRs.
2. **Hook denial handling** — if the harness `gh pr merge --admin` hook
   still denies, surface it succinctly ("hook blocked, want to add a
   permission rule, run yourself, or merge in UI?") rather than
   re-asking the merge decision itself. See `feedback_admin_merge_denial.md`.
3. **Phase 3 gate (`run / defer / refine`)** — DO ask this one. The
   user uses `defer` deliberately to hand the queue off to an external
   daemon (confirmed by the same session ending with `defer`). It is
   not noise — it is a real branching decision.
4. **Other autospec workflow gates** — bias toward auto-proceed when
   the SKILL.md has a clear default (e.g. `run` is the default for
   `/autospec`), unless the action is destructive or shared-state.
5. **Spec-amendment loop** — if the user adds scope mid-spec ("add this
   to the feature"), treat it the same: amend the design doc, open a
   second feat/spec-* PR, admin-merge, then continue. Do not stop and
   ask whether to amend.
6. **Brainstorm question budget** — the user reaffirmed (2026-05-02
   session) "i go with your defaults, just remember its supposed to be
   autonomous. So we really do not want to be involved unless its
   absolutely necessary or something destructive." Do NOT march through
   all 5 Phase-2 brainstorm questions when defaults are reasonable;
   collapse to a single confirm-defaults pass for low-stakes sections
   (error handling, testing strategy, naming) and only surface real
   branches (architecture, scope cuts, label semantics) for approval.
7. **Destructive remote actions ALWAYS require approval** — even when
   running autonomously, do NOT take any destructive remote action
   without an explicit user yes. Destructive remote = anything that
   irreversibly mutates shared state outside this machine. Examples
   that REQUIRE approval: deleting a remote branch (other than the
   automatic `gh pr merge --delete-branch` for an auto-implement PR
   already authorized in AGENTS.md), force-push to any branch, closing
   issues that the user did not file, deleting GitHub labels, deleting
   repos/projects, transferring ownership, posting to external services
   (Slack/email/etc.), publishing packages, modifying repo settings or
   collaborator permissions. Examples that do NOT need approval (already
   authorized): admin-merge of spec/auto-implement PRs, opening issues,
   adding labels, creating branches, opening PRs.

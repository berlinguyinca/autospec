---
name: Sync repo state before kicking off /autospec design phases
description: User prefers to merge upstream and resolve local drift before starting brainstorm/design phases of /autospec
type: feedback
wing: synthesis
drawer_class: lesson
originSessionId: 7205d05b-f2fd-4cde-9ced-ecc266a9bc7b
---
When the user invokes a multi-phase pipeline (e.g. `/autospec`) on a repo that has uncommitted local edits AND is behind origin/main, **pause Phase 1/2 and surface the git state first**. The user interrupted a Phase 1 brainstorm to say "merge the latest main into here" before answering scoping questions.

Why: Stale local edits frequently turn out to be earlier drafts of work that has since landed upstream — proceeding with brainstorm against a stale tree wastes tokens and produces obsolete proposals. Confirmed in 2026-04-30 session: 5 conflicting local edits all turned out to be inferior versions of changes already in origin/main.

How to apply: Before starting any /autospec investigation phase, run `git fetch && git status --porcelain && git rev-list --count HEAD..@{u}`. If there are uncommitted edits AND the branch is behind upstream, ask whether to merge first — don't auto-decide. Skip this gate only when the user explicitly says "ignore git state" or the repo is clean.

# worktree guard — never work on dirty branches; fresh worktrees enforced

- **Date:** 2026-06-03
- **Status:** Design (Phase 2)
- **Author:** berlinguyinca (refined via /autospec-refine)
- **Tracker target:** `berlinguyinca/autospec`

## Problem statement

The worktree rule exists only as prose (`skills/autospec-run/SKILL.md:609,729`)
— nothing enforces it. Verified gaps (2026-06-03):

- `scripts/dispatch-implementer.sh:78-82` **silently reuses an existing
  worktree** with no clean-check — a dirty-worktree hazard by design.
- **No script anywhere** checks "am I in the primary checkout"
  (`git-common-dir` vs `git-dir`) or "does this branch already exist on the
  remote" (`ls-remote --heads`).
- Live failure classes this session: the **#882/#886 collision** (a prior
  worker's branch + open PR existed; a new worker re-claimed and would have
  pushed a conflicting branch), recovery agents inheriting half-done worktrees
  (#917), stray `/tmp/ws-*` worktrees, and a perpetually-dirty primary
  checkout that every agent had to tip-toe around.

The watchdog GC (#887) covers orphaned-worktree *cleanup*; the missing piece
is the deterministic *preflight*.

## Goals (operator-decided)

- **G1:** A shared deterministic guard: agents NEVER work in the primary
  checkout or on a dirty/stale worktree — always a fresh (or verified-clean
  adopted) worktree off just-fetched `origin/main`.
- **G2 (PR-aware recovery ladder):** when the target branch already exists
  remotely: open PR → **validate + merge, no re-implementation** (the #886
  recovery); branch-only → **adopt in a fresh worktree and continue** (the
  #917 recovery); neither → create fresh.
- **G3 (scope: ALL git-mutating autospec flows):** run implementers first,
  then define spec-PRs, doc regenerate commits, explore sandbox, release.
  The primary checkout becomes **read-only territory for agents**; operator
  dirt in it never matters and is never touched.
- **G4:** Git best practices codified in AGENTS.md: fetch-before-branch,
  cleanup only after merge confirmed, `git worktree prune` in cleanup, no
  reuse of dirty worktrees (no force-push/amend already covered).

**Non-goals:** changing the watchdog GC; touching operator-driven git flows
outside autospec skills; rebase/merge-strategy changes.

## Design

### D1 — `scripts/worktree-guard.sh` (shared, repo-root scripts/, installed to `~/.autospec/scripts/`)

```
worktree-guard.sh assert                         # preflight in cwd
worktree-guard.sh resolve-branch --branch B --repo O/R   # the G2 ladder, JSON verdict
worktree-guard.sh create --branch B [--base origin/main] [--path /tmp/wt-B]
```

- **`assert`** (MUST pass before any file edit/commit; exit codes):
  - `3 in_primary_checkout` — `git rev-parse --git-dir` == `--git-common-dir`
    (cwd is the main checkout, not a linked worktree).
  - `4 dirty` — `git status --porcelain` non-empty (untracked included).
  - `5 stale_base` — after `git fetch origin <base>`: for a new branch, HEAD
    != `origin/main`; for an adopted branch, `merge-base HEAD origin/main`
    != `origin/main` tip is reported (warn-level by default, fail with
    `--strict-base`).
  - `2` usage / not a git dir. `0` ok.
- **`resolve-branch`** — deterministic ladder, no LLM:
  `gh pr list --head B --state open` → `{"state":"open-pr","pr":N}`;
  else `git ls-remote --heads origin B` non-empty →
  `{"state":"branch-only"}`; else `{"state":"fresh"}`. Exit 0 always; the
  caller branches on `state`.
- **`create`** — `git fetch origin` then `git worktree add -b B <path>
  origin/main` (or `add <path> origin/B` for adopt). If the worktree path
  already exists: reuse ONLY if clean AND on the same branch
  (else exit `4` with `code_health:worktree_dirty_reuse_refused`). Never
  silent-reuse a dirty tree (replaces dispatch-implementer's current
  behavior).

### D2 — dispatch-implementer.sh integration

`dispatch-implementer.sh` delegates worktree creation to `worktree-guard.sh
create` and calls `resolve-branch` first, surfacing the ladder verdict to the
orchestrator in its output contract. The silent-reuse path
(`dispatch-implementer.sh:78-82`) is removed. Existing
`tests/autospec-run/test_parallel_dispatch.bats` updated; new bats for the
refusal path.

### D3 — run trio + phase4-implementer.md wiring

`process(ISSUE)` step 1 becomes:
1. `resolve-branch` → on `open-pr`: skip implementation; check out the PR in
   a fresh worktree, run the issue's verification (tests + validate.sh) and
   the standard review loop against the EXISTING PR, then merge if green
   (#886 path). On `branch-only`: adopt the branch in a fresh worktree;
   continue remaining work (#917 path). On `fresh`: `create`.
2. `worktree-guard.sh assert` — MUST exit 0 before any edit; non-zero →
   comment + restore `auto-implement` + stop the issue (never "work around").
3. Prose hard rules added: implementers NEVER `cd` into the primary checkout,
   never `git checkout`/`commit` there; cleanup = `git worktree remove` after
   merge confirmed + `git worktree prune`.

Same assert instruction lands in `skills/autospec-run/prompts/phase4-implementer.md`.

### D4 — other git-mutating flows (phased)

Each flow gains the same pattern (resolve → create/adopt → assert):
- **autospec-define** spec-PR flow: the spec commit happens in a temp
  worktree (`/tmp/wt-spec-<slug>`), never in the primary checkout.
- **autospec-doc** regenerate commits: already on the PR branch in the
  implementer's worktree — add the `assert` call before committing.
- **autospec-explore** sandbox + **autospec-release**: assert before any
  commit step.

### D5 — AGENTS.md git best practices

New `## Git hygiene (agents)` section: fetch-before-branch; primary checkout
is read-only for agents; fresh-or-verified-clean worktrees only; cleanup after
merge + prune; the PR-aware ladder as the standard branch-exists behavior;
pointer to `worktree-guard.sh` as the enforcement tool.

## Sequencing constraint

D3 edits the autospec-run trio — serialize behind the cost-efficiency chain
(`#938→#939→#941→#942`, audit `#944`) AND any run-trio child of the
fix-routing spec if it has been decomposed by then. D4's define/doc/explore/
release edits serialize per-trio as usual. D1/D2 are dep-free.

## Testing & validation

- bats `tests/worktree-guard/`: every exit code; primary-checkout detection
  (fixture repo + linked worktree); dirty-reuse refusal; ladder verdicts with
  PATH-shadowed `gh`/`git ls-remote` mocks (per the repo's established mock
  pattern); adopt-path base check; idempotent `create`.
- dispatch-implementer bats updated (no silent reuse; verdict surfaced).
- Trio named-content checks in `validate.sh`: the assert step present in run
  trio + phase4-implementer.md; `## Git hygiene (agents)` present in
  AGENTS.md.
- Regression: existing parallel-dispatch + watchdog-GC tests stay green.

## Risks

| Risk | Mitigation |
|---|---|
| open-PR validate+merge path merges a bad stale PR | full verification (tests + validate.sh + review loop) runs against the PR before merge — same bar as fresh work |
| guard false-positives block work (e.g. transient fetch failure) | distinct exit codes + clear `code_health:*` identifiers; fetch failure = retry once then surface, never silently pass |
| adopted branch diverged badly from main | `assert` stale-base warn + the rebase-and-retest pre-merge gate already covers final state |
| trio conflicts with queued epics | D3 child gated on #944 (+ fix-routing run-trio child when filed) |

## Decomposition hint for /autospec-define

1. **`worktree-guard.sh`** (assert/resolve-branch/create + full bats). Dep-free.
2. **dispatch-implementer.sh integration** (remove silent-reuse; surface
   verdict; bats). Depends on 1.
3. **Run trio + phase4-implementer.md wiring** (ladder + assert + prose hard
   rules). Depends on 1, 2, **#944**, and the fix-routing spec's run-trio
   child if filed by then.
4. **Define spec-PR worktree + AGENTS.md git-hygiene section.** Depends on 1.
5. **Doc/explore/release asserts.** Depends on 1; split per-trio if caps require.
6. Standard Phase 5.5 audit issue depending on all.

> Decomposer notes: one trio per child; lock-step checkbox each; do NOT apply
> needs-autospec-template.

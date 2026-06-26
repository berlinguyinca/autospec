# watchdog claim-timeout — stop reclaiming live claims; make concurrent workers safe

- **Date:** 2026-06-25
- **Status:** Design (Phase 2)
- **Author:** berlinguyinca
- **Tracker target:** `berlinguyinca/autospec`

## Problem statement

autospec-run is *designed* for concurrent workers. Three layers make it safe:

1. **Atomic claim** — `claim-issue.sh` check-and-swaps `auto-implement →
   in-progress-by-bot` plus a marked GitHub `autospec-run-state` comment. That
   comment, **not** the local heartbeat file, is the authoritative
   cross-machine source of truth (SKILL.md: "The GitHub `autospec-run-state`
   comment written by these helpers is the cross-workstation source of truth.
   Local process heartbeat files remain useful for same-host progress … but
   they are not authoritative across machines.").
2. **Worktree isolation** — `dispatch-implementer.sh` + `worktree-guard.sh`
   give each issue its own `/tmp/wt-<branch>`.
3. **Per-session lock** — keyed by harness session id, so *separate sessions
   run independently by design*.

The watchdog undermines layer 1. `scripts/autospec-watchdog.sh:408` releases a
`claimed`-step heartbeat **purely on local heartbeat age** ≥
`AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS` (**default 300s**), with **no
cross-check against the authoritative GitHub run-state comment**:

```sh
# autospec-watchdog.sh:408 (local-heartbeat path)
if [ "$step" = "claimed" ] && [ "$age" -ge "$WATCHDOG_CLAIMED_TIMEOUT_SECS" ]; then
    reclaim_issue "$issue" "$age"          # <-- releases a possibly-LIVE claim
    claimed_released=$((claimed_released + 1))
    state_unset "$issue"; rm -f "$hb"; continue
fi
```

300s is **shorter than a normal `claimed → worktree_ready` transition**: a
fresh worktree off `origin/main` (clone/fetch + `git worktree add` on a large
repo) plus the implementer's first test-harness setup commonly exceeds five
minutes before the heartbeat advances past `claimed`. So the watchdog reclaims
a worker that is alive and working.

The GitHub-comment reconcile path (`reconcile_run_state_comments`,
`autospec-watchdog.sh:506`) is better — it skips open-PR steps
(`pr_created`/`awaiting_ci`) — but it still uses the **same too-short 300s** for
`claimed`, and the local-heartbeat path at line 408 bypasses GitHub entirely.

### Verified incident (2026-06-25, metabolomics-us/go-modules)

A sibling autospec-run session (separate harness session, same machine, shared
`~/.autospec` store) finished #1054 (PR #1087 merged 20:22Z) and **claimed
#1055 at 20:24Z** (local heartbeat `step:claimed`). A second session ran its
startup watchdog at 20:30Z, computed `age = 332s > 300s`, released the
heartbeat, and a follow-on label-reset flipped `in-progress-by-bot →
auto-implement` **out from under the live worker** — precisely the double-claim
the coordination layer exists to prevent. The claim was live; only the local
heartbeat age, never GitHub, was consulted.

## Goals (operator-decided)

- **G1:** Raise the default claimed-timeout above a realistic
  `claimed → worktree_ready` transition so normal implementations are never
  reclaimed. Default `300 → 1800` (30 min).
- **G2 (authoritative reclaim):** the local-heartbeat path must **not** release
  a `claimed` claim on local age alone. Before releasing, cross-check the
  GitHub `autospec-run-state` comment; only reclaim when GitHub **corroborates**
  staleness (run-state `updated_at` age ≥ threshold, step still `claimed`, no
  open PR) **or** when no valid run-state comment exists. A claim GitHub shows
  fresh is never released.
- **G3 (documented concurrency model):** SKILL.md gains a "Running concurrent
  workers" subsection — separate sessions, distinct `AUTOSPEC_WORKER_ID`, and
  the tuned watchdog env (`AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS`,
  `AUTOSPEC_WATCHDOG_RECLAIM_SECS`, `AUTOSPEC_WATCHDOG_STALE_SECS`) for
  long-running concurrent runs.

## Non-goals

- Redesigning the claim protocol or the `autospec-run-state` comment schema.
- Changing worktree isolation or the per-session lock.
- Touching the `pr_created`/`awaiting_ci` reconcile logic (already PR-aware).

## Design

- **D1 — raise the default.** In `scripts/autospec-watchdog.sh` set
  `WATCHDOG_CLAIMED_TIMEOUT_SECS="${AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS:-1800}"`.
  Update the header doc comment (line 14) and every doc reference in
  `skills/autospec-run/SKILL.md` and `AGENTS.md` that names the 300s default.

- **D2 — authoritative local-path gate.** Wrap the line-408 release in an
  authoritative cross-check. Reuse the existing `run_state_body_for_issue` /
  `extract_run_state_json` helpers (move them above the local loop, or factor a
  shared `claim_is_live_on_github <issue>` predicate). Release the `claimed`
  heartbeat **only if** one of:
  - no valid run-state comment exists for the issue, **or**
  - the comment's `updated_at` age ≥ `WATCHDOG_CLAIMED_TIMEOUT_SECS` **and**
    its step is still `claimed` **and** no open PR is attached.
  Otherwise SKIP (treat as live; do not reclaim, do not delete the heartbeat).
  GitHub-API failure (offline / rate-limited) is treated as "cannot prove
  stale" → **skip the release** (fail-safe: never reclaim a claim you can't
  prove is dead).

- **D3 — single source for the threshold.** Both the local-heartbeat path
  (line 408) and `reconcile_run_state_comments` (line 506) read the same
  `WATCHDOG_CLAIMED_TIMEOUT_SECS` constant so they never disagree.

- **D4 — docs.** Add the "Running concurrent workers" subsection to
  `skills/autospec-run/SKILL.md` near the session-lock section.

## Tests required (bats, no mocks of the unit under test)

Drive `autospec-watchdog.sh` with a fixture heartbeat dir and a stubbed `gh`
(PATH shim) returning canned run-state comment bodies:

1. **T1 — live, young:** `claimed` heartbeat `age < 1800s` → NOT released
   (`claimed_released=0`).
2. **T2 — old local, fresh GitHub:** `claimed` heartbeat `age > 1800s` but the
   run-state comment `updated_at` is fresh → NOT released (authoritative
   cross-check wins).
3. **T3 — old local, stale/absent GitHub:** `claimed` heartbeat `age > 1800s`
   and run-state comment stale (or absent) → released (`claimed_released=1`).
4. **T4 — gh failure is fail-safe:** `gh` shim exits non-zero → NOT released.
5. **T5 — default constant:** with no env override the effective claimed
   timeout is 1800.

### Primary smoke test

```bash
bats skills/autospec-run/tests/test_watchdog_claim_timeout.bats -f "T2"
```
(asserts an old-local but GitHub-fresh `claimed` claim is NOT reclaimed —
the exact incident this spec fixes).

## Files touched

- `scripts/autospec-watchdog.sh`
- `skills/autospec-run/SKILL.md`
- `AGENTS.md`
- `skills/autospec-run/tests/test_watchdog_claim_timeout.bats` (new)

## Acceptance criteria

- [ ] `WATCHDOG_CLAIMED_TIMEOUT_SECS` default is `1800`; env override still works.
- [ ] Local-heartbeat `claimed` release is gated on an authoritative GitHub
      run-state cross-check (G2/D2); a GitHub-fresh claim is never released.
- [ ] `gh` failure during the cross-check is fail-safe (no release).
- [ ] All five bats tests (T1–T5) pass.
- [ ] SKILL.md and AGENTS.md no longer document a 300s claimed default and
      include the "Running concurrent workers" subsection.

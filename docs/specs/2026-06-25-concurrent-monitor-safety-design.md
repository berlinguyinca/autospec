# Concurrent autospec monitor safety — design spec

**Date:** 2026-06-25
**Origin:** live incident — two autospec monitor sessions sharing the same repo
checkout + `~/.autospec` store on `metabolomics-us/go-modules` double-processed
issue #1055. A sibling session's watchdog reset #1055 from `in-progress-by-bot`
→ `auto-implement` while the owning worker was still live (heads-down in expand,
heartbeat step still `claimed`, age 332s > the 300s timeout).

## Goal

Make several autospec monitor processes drain the same repo (same machine,
shared `~/.autospec` store, or across machines) without colliding or
double-processing — by making the GitHub claim the sole cross-session authority,
never reclaiming a provably-live worker, and keying all shared state by one
canonical slug.

## Team personality

**Reliability/distributed-systems team** — orchestration engineer (watchdog +
claim CAS), platform/shell engineer (heartbeat store, slug keying, locks), test
engineer (bats with mocked `gh` + PID boundaries). Fits because the bug is a
distributed-claim liveness/authority problem, where the failure modes are
reclaiming a live owner and split-brain state.

**Review counter-team** — safety/operations lens: challenge "can two workers
still both believe they own an issue?", "can a genuinely-dead worker's claim
wedge the queue forever?", "does any reclaim path act on local state without
the GitHub authority?".

## Root cause (confirmed in code)

1. **Watchdog reclaims on local heartbeat age alone.**
   `scripts/autospec-watchdog.sh:408` — `if [ "$step" = "claimed" ] && [ "$age"
   -ge "$WATCHDOG_CLAIMED_TIMEOUT_SECS" ]; then reclaim_issue ...` — decides
   purely from the local heartbeat timestamp. It never reads the GitHub
   `autospec-run-state` comment, which the skill itself declares the
   cross-session source of truth.
2. **No process-liveness check.** The watchdog never checks whether the claiming
   worker's process is actually alive before reclaiming.
3. **`claimed`-timeout (300s) is shorter than the claim→worktree_ready gap.**
   Expand + pattern survey routinely exceed 5 min, so a live worker still at
   step `claimed` is flagged dead.
4. **Slug-normalization split.** The heartbeat store contains both
   `berlinguyinca_autospec` and `berlinguyinca-autospec` (and the tidyboard
   equivalent) — `/` is normalized inconsistently (`_` vs `-`), so heartbeats and
   run-state for one repo fragment across directories. Echoes
   `feedback_heartbeat_cross_repo_collision`.
5. **Per-session lock can't see siblings.** `autospec-run-session-lock.sh` is
   keyed by harness session id (intentionally — separate sessions run
   independently). It does not protect two sibling sessions sharing one repo
   checkout + store.

## Design

### F1 — Watchdog honors the GitHub claim authority (core fix)
Before reclaiming ANY `claimed`/in-flight heartbeat, the watchdog must read the
issue's `autospec-run-state` comment (`claim-issue.sh`/`release-issue.sh` already
write it). Decision table for a heartbeat older than the timeout:

| GitHub run-state | Action |
|---|---|
| absent / `released` / `failed` | reclaim (no live owner) |
| `claimed`/in-flight, **different** worker, GitHub `ts` fresh (< RECLAIM window) | **DO NOT reclaim** — live sibling |
| `claimed`/in-flight, GitHub `ts` stale (≥ RECLAIM window) | reclaim + comment |
| owned by **this** worker | refresh, never reclaim |

The local heartbeat age becomes a *trigger to check*, not the decision. The
GitHub `ts` + `worker_id` are authoritative.

### F2 — Process-liveness short-circuit (same host)
The heartbeat + run-state already carry `worker_id` (`host:user:harness:pid`).
When the heartbeat's host == this host, check `kill -0 <pid>` (or equivalent): if
the worker process is alive, never reclaim regardless of age. Cross-host falls
back to the F1 GitHub-freshness rule. A new `worker-liveness.sh` helper
encapsulates the host-match + PID-liveness check (pure, testable).

### F3 — Liveness window, not step-timeout
Replace the bare 300s `claimed`-step timeout with a freshness window keyed off
the **last heartbeat/run-state refresh**, and have the implementer refresh its
heartbeat at the start of the expand phase (the current claim→worktree_ready gap
is the unguarded window). Default `AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS` raised
to a safe value (e.g. 1800) and only acted on after the F1/F2 authority+liveness
checks clear.

### F4 — Canonical slug helper
One `repo-slug.sh` helper that maps `owner/name` → a single canonical form
(pick one: `owner__name` or `owner-name`) used EVERYWHERE shared state is keyed:
heartbeat dirs, run-state lookups, lock files, cycle counters. Migrate existing
readers/writers to it; on read, also accept the legacy alternate form for one
release so in-flight heartbeats aren't orphaned.

### F5 — Repo-scoped advisory lock (shared-checkout guard)
Add an OPTIONAL repo-scoped advisory lock (keyed by canonical slug, not session
id) for the same-machine shared-checkout+store case, acquired around the
claim+worktree-create critical section. Off by default (the GitHub claim CAS +
worktree isolation already prevent most collisions); opt-in via
`AUTOSPEC_REPO_LOCK=1` for operators running multiple monitors against one
checkout. Never blocks cross-host workers (they don't share the lock dir).

## Decomposition preview (≈5 children + epic + audit)
1. EPIC umbrella.
2. `repo-slug.sh` canonical slug helper + bats (F4 core, no deps).
3. `worker-liveness.sh` host-match + PID-liveness helper + bats (F2, no deps).
4. `autospec-watchdog.sh` GitHub-authority + liveness reclaim gate + bats (F1+F2+F3; depends on #2,#3). Mock `gh` and PID boundary.
5. Implementer heartbeat-refresh-at-expand + raised timeout default — autospec-run trio + goldens (F3; depends on #4).
6. Optional repo-scoped advisory lock `autospec-repo-lock.sh` + bats + opt-in wiring (F5; depends on #2).
7. Phase 5.5 audit + remediation.
Each ≤400 words, ≤3 logical units, `reasoning:medium`/`ctx:64k` (the watchdog one may be `reasoning:deep`).

## Tests required
TDD throughout. `gh` is an external boundary → mock it (PATH stub) per project
convention; PID liveness mocked via known-dead/known-alive pids. No internal
mocks. Each helper green in isolation; `validate.sh` green on the integrated
tree.

## Self-review
- **Placeholders:** none.
- **Consistency:** F1 makes GitHub authoritative; F2 is a same-host fast-path
  that only ever *prevents* reclaim (never causes one); F3 widens the window;
  F4/F5 are keying/locking. No two features contradict.
- **Scope:** one multi-issue pipeline; F5 is optional/opt-in and isolatable.
- **Critical risk:** a genuinely-dead worker whose GitHub `ts` is recent could
  wedge the queue (false-negative reclaim). Mitigated by the RECLAIM window
  (cross-host) + PID-liveness (same-host): a dead same-host worker fails the
  `kill -0` check and IS reclaimed; a dead cross-host worker is reclaimed once
  its GitHub `ts` ages past the window. Document the window trade-off.
- **On merge:** note the incident (go-modules #1055) as the regression this
  closes; add a regression test that reproduces "live worker, age>timeout, fresh
  GitHub claim → NOT reclaimed".

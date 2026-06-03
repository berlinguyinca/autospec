# Crash-resume for interrupted autospec runs

- **Date:** 2026-06-03
- **Status:** Design (Phase 2)
- **Author:** autospec Phase 2 design-spec author (autonomous)
- **Tracker target:** `berlinguyinca/autospec`

## Problem statement

When an autospec implementation run is interrupted by a **host crash, a session
crash, or the operator's terminal dying**, the run stops with no auto-restart.
Every existing recovery mechanism in autospec assumes a *live process* is still
running to read durable state; none of them re-reads that durable state on a
*fresh* start. The run's durable footprint survives the crash, but nothing
consumes it, so the run simply stalls until a human notices.

**What survives a crash (durable):**

- GitHub run-state comment per issue — `{schema:1, repo, issue, worker_id,
  state, branch, pr, step, paths, claimed_at, updated_at, ttl_seconds}`
  (cross-machine).
- GitHub labels: `auto-implement`, `in-progress-by-bot`, `paused-by-user`.
- Machine-local heartbeat files
  `~/.autospec/process-heartbeats/<repo-slug>/<issue>.json`.
- Pushed branch commits / open PRs on the remote.

**What stalls (lost on crash):**

- The top-level orchestrator session that relaunches the stateless monitor each
  batch — dead, so the relaunch loop never fires again.
- The tmux Python context-monitor daemon and the usage-limit `nohup` daemon —
  both die with the machine on reboot.
- Uncommitted mid-issue work in the per-issue worktree `/tmp/wt-<branch>`
  (verified `skills/autospec-run/SKILL.md:564-565,685,932`) — never WIP-committed
  unless the operator ran `/autospec-stop --immediate`.
- Orphaned `/tmp/wt-*` worktrees — never garbage-collected.

## Already-handled / scope-out inventory

These are **out of scope** — do not reinvent and do not add a second lock.
Paths and line numbers re-confirmed against the working tree on 2026-06-03.

| Mechanism | Location (confirmed) | Why it does not close the gap |
|-----------|----------------------|-------------------------------|
| GitHub run-state comment (durable, cross-machine; atomic CAS, lowest-comment-id wins, loser self-cleans duplicates) | `skills/autospec-run/scripts/run-state.sh:148-189` (schema build at `:165`; duplicate-id cleanup at the `for duplicate_id ...` loop) | Stores state; nothing re-reads it on a fresh start. **Resume reuses this lock — never adds another.** |
| Heartbeats (machine-local) | `skills/autospec-run/scripts/heartbeat-write.sh:80-87` (writes `{issue,branch,step,ts,pr,repo}`) | Local only; no consumer on cold start. **Note: schema has NO `host` field today** (verified — grep for `host` returns nothing). |
| Labels-as-state | `auto-implement` / `in-progress-by-bot` / `paused-by-user` | Passive; no process flips them back on crash except the watchdog, which itself needs a live monitor. |
| Watchdog reclaim | `scripts/autospec-watchdog.sh` (`WATCHDOG_CLAIMED_TIMEOUT_SECS=300` `:23`; `WATCHDOG_RECLAIM_SECS=10800` `:22`; swap `in-progress-by-bot`→`auto-implement` at `:174-175`; server `updated_at` age math at `:386-405`) | Re-queues an *issue*; runs only inside a live monitor loop; does not restart the monitor or resume partial work; does not GC `/tmp/wt-*`. |
| Stateless monitor relaunch | `skills/autospec-run/SKILL.md:263-302` (`~/.autospec/batch-done.json`; missing = "crashed, safe to relaunch") | Works only while the **top-level orchestrator session** is alive. |
| `/autospec-stop --resume` | `skills/autospec-stop.sh` (keys off `paused-by-user`, classifies via `detect-monitor-exit-mode.sh`) | User-invoked, intentional-stop recovery; not crash recovery; needs a human. |
| Usage-limit supervisor | `scripts/autospec-usage-limit.sh` (durable state `~/.autospec/usage-limits/` `:10`; arms `nohup` daemon `:175`; re-execs stored `command` `:227`) | Sole shell-only auto-relaunch, but for **quota pauses**; its `nohup` daemon dies on reboot. |
| Context-monitor rollover | `packages/autospec_context_monitor/engine.py:60-90` | Token-% rollover only; tmux daemon dies with session/machine. |
| `/autospec-continue` | skill | Harvests *next-step* work; does not resume crashed in-flight work. |

## Root cause

**Every recoverer needs a live process.** Durable state (GitHub comments,
labels, heartbeats, pushed branches) survives any crash, but the only code that
reads it — the monitor loop, the watchdog, the usage-limit daemon, the context
daemon — is hosted inside a process that dies with the crash. There is:

1. No **external supervisor** (launchd / systemd / `@reboot` cron) that re-runs
   the run after a host or session crash.
2. No **detect-and-continue entrypoint** that scans durable run-state +
   heartbeats on a *fresh* start and continues the run.
3. No **durable capture** of the relaunch command. `AUTOSPEC_RESUME_COMMAND` is
   a session env var (`skills/autospec-run/SKILL.md:244`); it is persisted to a
   durable file **only** when a usage-limit pause arms the daemon
   (`scripts/autospec-usage-limit.sh:152-173`). A bare host/session crash never
   triggers that path, so **the relaunch command is lost on reboot.** (Critical
   finding — see Phase-2 critical-improvement check.)
4. No **registry of active runs** for a supervisor to consult on boot — the
   supervisor would not know *which* repo(s)/run to resume.
5. No **orphaned-worktree GC**; `/tmp/wt-*` leaks accumulate.

## Goal

On a fresh start after a crash, autospec **detects an interrupted run from
durable run-state + heartbeats and auto-continues it**, without adding a second
lock, without stealing a genuinely-live worker on another host, without
deleting un-pushed work, and without thrashing — capped at a bounded number of
consecutive auto-resume attempts before halting and surfacing to the operator.

## Team personality — Reliability / backend

This is distributed-systems reliability work on a crash boundary: the unit of
correctness is "exactly one claim survives a concurrent restart, and no live
work is stolen or destroyed." The fitting team:

- **Backend developer** — owns the new `/autospec-resume` skill trio and the
  shell scan logic that reuses `run-state.sh` reads.
- **Platform engineer** — owns the cross-platform supervisor install surface
  (launchd plist on macOS, systemd unit on Linux, `@reboot` cron fallback) and
  the durable run registry.
- **SRE** — owns the crash-vs-live decision (watchdog windows off **server**
  `updated_at`), the attempt-cap back-off, and the "exit 0, do nothing" safe
  default.
- **Security advisor** — owns the supervisor's least-privilege footprint and
  ensures the registry cannot be poisoned into running an arbitrary command.
- **Distributed-systems / concurrency specialist** — owns idempotency: two
  concurrent resumes (supervisor + human; or two hosts) must collapse to one
  claim via the existing GitHub CAS lock.

**Risks this team must notice:** double-resume, live-worker theft, GC deleting
un-pushed work, supervisor boot-thrash, cross-host partial-resume of a worktree
on a dead machine's `/tmp`, and a non-durable relaunch command. Push every one
of these into the children as acceptance criteria with bats coverage.

## Review counter-team — security + operations + data-integrity

A second-pass review team challenges these assumptions (stay strictly
in-scope — no Phase 1/2/3 checkpointing):

- **Security reviewer** — "Can the run registry or `AUTOSPEC_RESUME_COMMAND` be
  edited to make the supervisor execute an attacker-chosen command on boot?"
  Demand the supervisor only ever re-runs a command it itself captured for a run
  it can independently confirm is open via GitHub.
- **Operations reviewer** — "Does the supervisor thrash on every boot when there
  is no open run? Does it back off? Is the install/uninstall idempotent across
  launchd/systemd/cron?"
- **Data-integrity reviewer** — "Does GC ever delete a worktree with un-pushed
  commits? Does resume ever add a second lock? Does cross-host partial-resume
  read a `/tmp` that does not belong to this host?"

## Architecture — where the code lives

```
skills/autospec-resume/              # NEW skill (lock-step trio + standard scaffolding)
  SKILL.md                           #   byte-identical body across the trio
  opencode/agent.md                  #   (frontmatter differs only)
  codex/prompt.md                    #   (needs leading blank line before body)
  install.sh  uninstall.sh  README.md
  validate.sh                        #   per-skill structural check
  scripts/
    resume-scan.sh                   #   scan durable run-state + heartbeats; decide
    resume-attempts.sh               #   durable consecutive-attempt counter (sentinel)
scripts/
  autospec-run-registry.sh           # NEW: durable registry of active runs (child 1 writer; child 2 reader)
  autospec-supervisor.sh             # NEW (child 2): boot entrypoint that invokes /autospec-resume
  autospec-supervisor-install.sh     # NEW (child 2): launchd | systemd | @reboot cron install/uninstall
  autospec-watchdog.sh               # EXTEND (child 3): orphaned /tmp/wt-* GC pass
scripts/validate.sh                  # EXTEND: structural checks for all of the above
tests/resume/                        # bats fixtures with PATH-shadow gh/launchctl/systemctl mocks
```

ROI: each component has a named consumer today — `/autospec-resume` is consumed
by the supervisor and by a human typing `/autospec-resume`; the registry is
consumed by the supervisor; the GC pass is consumed by the existing watchdog
loop. Nothing is forked; we extend `run-state.sh` reads, the watchdog, and the
usage-limit durable-capture pattern.

## Interactivity / API shape

**`/autospec-resume [--resume-partial] [--repo <owner/name>] [--dry-run]`**

- Default (no flags): scan the current repo's durable run-state + heartbeats,
  apply the auto-resume pre-conditions, and if all pass, relaunch the run via
  the durably-captured command (clean-restart off `origin/main`).
- `--resume-partial`: additionally re-attach to the existing `/tmp/wt-<branch>`
  **only when** `heartbeat.host == $(hostname)`; otherwise silently fall back to
  clean-restart.
- `--dry-run`: print the decision and the command that *would* run; change
  nothing; exit 0.
- `--repo`: target a specific repo (used by the supervisor, which iterates the
  registry).

**Supervisor:** `autospec-supervisor-install.sh {install|uninstall|status}`
selects launchd / systemd / cron by platform. The boot unit runs
`autospec-supervisor.sh`, which reads the registry and calls `/autospec-resume
--repo <r>` for each registered open run.

**Exit codes & output (resume):**

| Code | Meaning | Prints |
|------|---------|--------|
| 0 | Nothing to resume, or `--dry-run` | one-line reason (`no open in-progress run-state`, `paused-by-user present`, `stop.flag present`, `all issues closed`, `attempt cap reached`) |
| 0 | Resumed | `resuming <repo>: <N> issue(s); command=<...>` |
| 1 | Hard error (bad args, no `gh`) | error to stderr |

## Data model

**Reused unchanged:** run-state schema (`run-state.sh:165`) and heartbeat schema
— **except** child 1 adds a `host` field to the heartbeat schema (required for
the cross-host partial-resume gate; the field is absent today). Adding a field
is backward-compatible: readers treat a missing `host` as "unknown host" →
never eligible for `--resume-partial`.

**New: run registry** `~/.autospec/active-runs/<repo-slug>.json` (path-scoped by
repo-slug, mirroring the heartbeat collision lesson):

```json
{
  "schema": 1,
  "repo": "owner/name",
  "repo_dir": "/abs/path/to/checkout",
  "harness": "claude|codex|opencode",
  "resume_command": "<exact non-interactive relaunch command>",
  "host": "<hostname that registered this run>",
  "registered_at": "<iso8601>",
  "updated_at": "<iso8601>"
}
```

The registry is written by `/autospec-run` at monitor-launch time (child 1
wires the call in) using the **same** command it already computes for
`AUTOSPEC_RESUME_COMMAND` (`SKILL.md:244`) — this makes the relaunch command
**durable across reboot**, closing root-cause #3. Written two-line atomic
temp+mv; entries older than 24h with no matching open in-progress run-state are
ignored/pruned.

**New: resume-attempt counter** `~/.autospec/resume-attempts/<repo-slug>.json`:

```json
{ "schema": 1, "repo": "owner/name", "count": 0,
  "first_at": "<iso8601>", "updated_at": "<iso8601>" }
```

Two-line atomic temp+mv; stale > 24h ignored (sentinel convention). Incremented
before each auto-resume relaunch; reset to 0 when a batch makes forward progress
(any issue reaches `merged`). At `count >= AUTOSPEC_RESUME_MAX_ATTEMPTS`
(default 3) resume halts and surfaces.

## Error handling (adversarial requirements)

- **Idempotency / no second lock.** Resume relaunches the *run*; the relaunched
  monitor claims issues through the **existing** GitHub CAS lock-comment
  (`run-state.sh`, lowest-comment-id wins, loser self-cleans). Resume itself
  never writes a run-state comment and never adds any new lock. Two concurrent
  resumes (supervisor + human, or two hosts) therefore converge to exactly one
  claim per issue at the existing CAS boundary.
- **Auto-resume pre-conditions (ALL required, else exit 0, no relaunch):**
  (a) ≥1 issue with run-state labeled `in-progress-by-bot` whose heartbeat step
  ∉ {`merged`,`failed`}; AND (b) no `~/.autospec/stop.flag`; AND (c) no issue
  labeled `paused-by-user`; AND (d) not all issues closed.
- **Crash-vs-live.** Treat an issue as crashed (eligible) only when its age,
  computed from run-state **server** `updated_at` (never a local clock —
  mirrors `autospec-watchdog.sh:386-405`), satisfies `step=claimed &&
  age>=300` OR `age>=10800`. Otherwise assume a live/slow worker elsewhere and
  do **not** steal.
- **Cross-host.** `--resume-partial` re-attaches to `/tmp/wt-<branch>` only when
  `heartbeat.host == $(hostname)`. Cross-host (or missing `host`) MUST
  clean-restart off `origin/main` — the crashed worktree is on a dead machine's
  local `/tmp` and is unreachable.
- **GC safety.** Prune a `/tmp/wt-*` worktree only when ALL hold: the branch has
  **no un-pushed commits** (`git -C <wt> log --not --remotes` empty), AND its
  issue is closed or unlabeled, AND no live heartbeat references it. Use `git
  worktree remove --force` only after these pass; never `rm -rf` a worktree with
  un-pushed commits.
- **Supervisor safety.** On boot the supervisor acts only on registry entries
  with a confirmed open in-progress run-state; otherwise exit 0 and back off
  (do not re-arm immediately). Install surface is idempotent per platform.
- **Attempt cap.** After `AUTOSPEC_RESUME_MAX_ATTEMPTS` (default 3) consecutive
  auto-resume attempts without forward progress, halt, print the cap-reached
  reason, and surface to the operator — never infinite-loop.

## Testing — validation via shell only

This repo has no language test runner; validation is shell + bats with
PATH-shadow subprocess mocks (per AGENTS.md). Plan:

- **`scripts/validate.sh` structural checks:** `skills/autospec-resume/` exists
  with the lock-step trio + `install.sh`/`uninstall.sh`/`README.md`/per-skill
  `validate.sh`; the trio passes `check_lockstep`; SKILL.md carries the
  `## Startup self-update` section, the harness-detection / Subagent-model-tier
  section, and the `## Required capabilities & harness adapter` row (the
  standard new-skill structural sections); `bash -n` on every new script; the
  registry + supervisor + GC scripts exist and are executable.
- **bats fixtures (`tests/resume/`)** with PATH-shadow mocks for `gh`,
  `launchctl`, `systemctl` (and `hostname`):
  1. **Idempotency:** simulate two concurrent `/autospec-resume` invocations
     against the same issue; assert exactly one claim survives at the CAS lock
     (resume writes no run-state comment itself).
  2. **Pre-conditions:** each of stop.flag-present, paused-by-user-present,
     all-closed, and no-in-progress independently yields exit 0 + no relaunch.
  3. **Crash-vs-live:** `updated_at` age < threshold → not stolen; age ≥
     threshold → eligible. Mock `gh` to return controlled server `updated_at`.
  4. **Cross-host:** heartbeat `host != hostname` (and missing `host`) under
     `--resume-partial` → clean-restart, not partial.
  5. **GC safety:** a worktree with un-pushed commits is NOT pruned; an
     in-progress-this-host worktree is NOT pruned; a clean closed-issue worktree
     IS pruned.
  6. **Supervisor thrash:** no open run in registry → supervisor exits 0,
     re-arm not called.
  7. **Attempt cap:** counter at max → exit 0 cap-reached, no relaunch; counter
     resets on a merged issue.

**Phase-2 critical-improvement check — "what else fails even if this spec and
its tests pass?"** The highest-risk answer is durability of the relaunch
command (root-cause #3): if `AUTOSPEC_RESUME_COMMAND` is never written to a
durable file, the supervisor has nothing to run after reboot and the whole
feature silently no-ops. **Mitigation folded in:** child 1 makes `/autospec-run`
write the registry (with `resume_command`) at monitor launch, and an acceptance
criterion + bats test assert that after a simulated reboot (fresh process, env
cleared) the registry alone yields a runnable command and the supervisor reads
the *registry-derived* command, not the env var. Second-highest risk is the
supervisor not knowing *which* runs to resume — answered by the registry being
the authoritative per-repo list the supervisor iterates (root-cause #4), with a
bats test that a two-repo registry resumes exactly the repos with open run-state.

## Acceptance criteria

- [ ] `/autospec-resume` exists as a lock-step trio
  (`SKILL.md`/`opencode/agent.md`/`codex/prompt.md` byte-identical bodies;
  `codex/prompt.md` has its leading blank line) and passes
  `validate.sh check_lockstep`.
- [ ] The new skill ships `install.sh`, `uninstall.sh`, `README.md`, a per-skill
  `validate.sh`, the `## Startup self-update` section, the harness-detection /
  Subagent-model-tier section, and the `## Required capabilities & harness
  adapter` row; `needs-autospec-template` is NOT applied to it.
- [ ] Resume adds **no** new lock; idempotency bats test proves two concurrent
  resumes converge to exactly one claim via the existing GitHub CAS lock.
- [ ] All four auto-resume pre-conditions enforced; each failing condition yields
  exit 0 + no relaunch (bats).
- [ ] Crash-vs-live decision uses run-state **server** `updated_at`; a fresh
  (age < 300) claimed issue is never stolen (bats).
- [ ] `--resume-partial` re-attaches only when `heartbeat.host == hostname`;
  cross-host / missing-host clean-restarts (bats); heartbeat schema gains a
  `host` field, backward-compatible.
- [ ] Orphaned-worktree GC prunes only clean (no un-pushed commits), closed/
  unlabeled, no-live-heartbeat worktrees; un-pushed-commit and
  in-progress-this-host worktrees are NOT pruned (bats).
- [ ] `AUTOSPEC_RESUME_COMMAND` is durably persisted to
  `~/.autospec/active-runs/<repo-slug>.json` at monitor launch; after a
  simulated reboot the supervisor derives a runnable command from the registry
  alone (bats).
- [ ] Supervisor acts only on registry entries with a confirmed open in-progress
  run-state; with no open run it exits 0 and does not re-arm (bats); install/
  uninstall idempotent on launchd, systemd, and `@reboot` cron.
- [ ] Consecutive auto-resume attempts capped at `AUTOSPEC_RESUME_MAX_ATTEMPTS`
  (default 3); at the cap, halt + surface; counter resets on forward progress
  (any issue `merged`) (bats).
- [ ] `scripts/validate.sh` extended with structural + bash-syntax checks for
  the new skill, registry, supervisor, and GC scripts; full `validate.sh`
  passes.

## Out of scope

- **Phase 1/2/3 checkpointing** (research / spec / decompose crash mid-flight has
  zero checkpoint) — deferred to a follow-up tracker; not in v1.
- Any **new lock** or replacement of the GitHub CAS run-state lock.
- Cross-host *partial* resume of a worktree on another machine's `/tmp` (always
  clean-restart there).

## Decomposition preview

**Parent tracker:** "Crash-resume for interrupted autospec runs" — links the
three ordered children below; each is ≤3 files, ≤30-line step outline, ≤400-word
body, byte-identical trio where a skill is touched.

**Child 1 — `/autospec-resume` skill + durable run registry (NEW skill).**
Create the `skills/autospec-resume/` lock-step trio + `install.sh`/
`uninstall.sh`/`README.md`/per-skill `validate.sh`, **including the standard
new-skill structural sections** (Startup self-update block, harness-detection /
Subagent-model-tier section, Required-capabilities adapter row); do NOT apply
`needs-autospec-template`. Add `scripts/resume-scan.sh`,
`scripts/resume-attempts.sh`, `scripts/autospec-run-registry.sh`; extend the
heartbeat schema with `host`; wire `/autospec-run` to write the registry (durable
`resume_command`) at monitor launch. Implements pre-conditions, crash-vs-live
(server `updated_at`), idempotency (no new lock), cross-host gate, attempt cap.
bats: idempotency, pre-conditions, crash-vs-live, cross-host, attempt cap.
*(NEW-SKILL structural requirement explicitly noted.)*

**Child 2 — External boot supervisor.** Add `scripts/autospec-supervisor.sh`
(reads the registry, calls `/autospec-resume --repo <r>` per open run, exits 0 +
backs off otherwise) and `scripts/autospec-supervisor-install.sh`
(`install|uninstall|status` selecting launchd plist / systemd unit / `@reboot`
cron by platform). bats with PATH-shadow `launchctl`/`systemctl` mocks:
no-open-run → exit 0 no re-arm; two-repo registry resumes exactly the open ones;
install/uninstall idempotent. Depends on child 1's registry + skill.

**Child 3 — Watchdog orphaned-worktree GC.** Extend `scripts/autospec-watchdog.sh`
with a GC pass that prunes `/tmp/wt-*` only when the branch has no un-pushed
commits AND the issue is closed/unlabeled AND no live heartbeat references it.
bats: un-pushed-commit worktree NOT pruned; in-progress-this-host NOT pruned;
clean closed-issue worktree pruned. Extends `validate.sh` accordingly.

## Autonomous assumptions

> AUTONOMOUS ASSUMPTION: `worker_id` in run-state does not reliably encode the
> hostname, so a dedicated `host` field is added to the heartbeat schema for the
> cross-host gate rather than parsing `worker_id`. Verified the heartbeat schema
> has no `host` field today; `worker_id` format was not audited for hostname
> content.

> AUTONOMOUS ASSUMPTION: `AUTOSPEC_RESUME_MAX_ATTEMPTS` default = 3 and the
> registry/counter staleness window = 24h, matching the existing sentinel
> convention. No existing constant was found dictating these exact values.

> AUTONOMOUS ASSUMPTION: the supervisor's cross-platform install surface is
> launchd (macOS) / systemd user-or-system unit (Linux) / `@reboot` cron
> fallback. The exact privilege level (user vs system) is left to child 2 to
> choose least-privilege; security review must confirm the supervisor only
> re-runs a command it captured for a run it independently confirms open.

> AUTONOMOUS ASSUMPTION: the registry is written by `/autospec-run` at monitor
> launch reusing the command it already computes at `SKILL.md:244`. If the
> harness sets `AUTOSPEC_RESUME_COMMAND` externally, child 1 still mirrors it
> into the registry so the durable path does not depend on an env var surviving
> reboot.

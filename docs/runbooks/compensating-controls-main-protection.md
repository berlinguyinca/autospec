# Compensating controls for the absent `main` branch protection

GitHub branch protection (required status checks, no direct pushes, signed
commits only) is **not** configured on `main` in this repository. A commit that
breaks the workspace build can therefore land on `main` and stay there until a
human notices. This happened twice in a row (issue #4108; the second incident
was #4098), because the three gates that should have caught it were all absent:

1. No dedicated per-commit "main builds" check — the long `build-test` job was
   the only signal, and on `main` it raced itself until #4107 excluded `main`
   from cancel-in-progress.
2. Merge automation never read a local build exit status — it trusted remote
   CI state, which could be `pending` or stale.
3. When `main` was red, nobody knew *which commit* broke it without scrolling
   through Actions history by hand.

This document is the compensating-control register: the automation that stands
in for branch protection, what each control does, and the operator runbook for
when `main` breaks anyway.

## The controls

| # | Control | Where | Failure mode it prevents |
|---|---------|-------|--------------------------|
| 1 | **`main-builds` CI job** — dedicated 30-minute `cargo build --workspace --all-targets` check on every `main` commit (`.github/workflows/rust.yml`). Fast enough to finish before the next merge, and `main` is exempt from cancel-in-progress (#4107), so it actually completes. | `.github/workflows/rust.yml` → `main-builds` | Build breakage landing on `main` with no fast per-commit signal. |
| 2 | **Local buildability gate in merge automation** — `scripts/autospec-guarded-merge.sh` runs `cargo build --workspace --all-targets` against the exact PR head OID **before** the admin merge and reads the process exit status. A non-zero exit blocks the merge (`blocked local_build_failed`, exit 1). Default-on; infrastructure problems (no `cargo`, not a git worktree, OID not local) fail open to the CI gate, but an actual build failure fails closed. | `scripts/autospec-guarded-merge.sh` → `_local_build_gate` (stage 4, before the CI-conclusion gate) | A merge proceeding while the build is red, because remote CI state was pending/stale/advisory. |
| 3 | **Automated first-failing-commit report** — when the main-health monitor halts, `scripts/first-failing-commit.sh` identifies the first `main` commit after the last green `main-builds` run and emits `FFC:FIRST_FAILING=<sha>` + subject lines on the halt path. | `scripts/autonomous-resilience.sh` → `cmd_main_health` halt path; `scripts/first-failing-commit.sh` | Red `main` with the breaking commit unknown, so the fix takes minutes of manual Actions archaeology. |
| 4 | **main-health monitor** — `autospec resilience main-health` polls `main` CI on every monitor tick and halts Tier-1 merges (`DECISION:halt`, exit 1) while `main` is red. Advisory checks (`AUTOSPEC_MAIN_HEALTH_IGNORE_CHECKS`) stay advisory; real failures halt. | `scripts/autonomous-resilience.sh` → `cmd_main_health` | A red `main` silently accumulating further merges. |
| 5 | **Concurrency exemption for `main`** — cancel-in-progress applies to PR refs only, so queued `main` runs are never cancelled by newer ones. | `.github/workflows/rust.yml` → `concurrency` (issue #4107) | `main` CI runs that never complete, leaving the status of `main` permanently unknown. |

The controls are layered: (5) makes the per-commit signal trustworthy, (1)
produces it, (2) stops a new red merge at the gate, (4) stops *further* merges
once `main` is already red, and (3) tells the operator exactly what to revert.

## Operator runbook: `main` is red

The main-health monitor halts Tier-1 merges and the halt output carries the
first-failing-commit report:

```
DECISION:halt
CI_STATE:failure
CHECK_RUNS:6
IGNORED_CHECK_RUN_FAILURES:0
FFC:CHECK:main-builds
FFC:STATE:broken
FFC:LAST_GOOD:9f3a1c2…
FFC:FIRST_FAILING:4b7e9d0…
FFC:FIRST_FAILING_SUBJECT:feat: something that broke the build
```

1. **Confirm.** `git log -1 4b7e9d0` (the `FIRST_FAILING` sha) and open the
   `main-builds` run for that commit to read the compiler output.
2. **Fix, in order of preference:**
   - **Revert** if the commit is recent, self-contained, and its author is
     available: `git revert 4b7e9d0` on a branch, PR it through
     `scripts/autospec-guarded-merge.sh` (the local build gate in control 2
     proves the revert builds before the admin merge).
   - **Fix forward** if reverting would strand dependents: open a fix PR the
     same way. The fix PR's `main-builds` check runs against its head, but the
     *merge* only proceeds once the local gate and the main-health monitor both
     agree `main` is green again — so rebase the fix onto a green `main` first,
     or land a minimal revert, then the fix.
3. **Un-halt.** Merges resume automatically on the next monitor tick that sees
   `DECISION:continue`; there is no latch to clear. If the break was an
   advisory-only check (a release-publish check, etc.), prefer adding it to
   `AUTOSPEC_MAIN_HEALTH_IGNORE_CHECKS` over a revert.
4. **Post-mortem line.** If `main` stayed red longer than one merge window,
   record which control failed to fire (and why) in the incident issue — the
   controls above are only as good as their failure modes being observable.

## Manual checks (no automation)

```sh
# Is the workspace buildable right now?
cargo build --workspace --all-targets

# Which main commit first broke main-builds?
scripts/first-failing-commit.sh --repo OWNER/REPO

# What does the monitor decide about main right now?
scripts/autonomous-resilience.sh main-health --repo OWNER/REPO
```

## Why not just enable branch protection?

The deploy target for this automation (self-hosted runners, fork-based PR
flow, admin-merge authority for `auto-implement` PRs) does not currently
support required-status-check enforcement the way the controls assume a
hosted repo does; the admin-merge path in particular is *designed* to merge
without host-side gating, which is exactly why control 2 must read the build
exit status itself. If branch protection becomes available, controls 1–2
remain (defense in depth) and this document's runbook is the fallback for the
gap between "check required" and "check actually verified the build".

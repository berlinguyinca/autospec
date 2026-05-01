# Runbook — `needs-classify` sweep

Periodic sweep that promotes listener-filed issues from the
`needs-classify` bucket into the `auto-implement` queue. Pairs with
spec §5.3 and the `## Listener-filed issues lifecycle` section of
`AGENTS.md`.

## Overview

Issues filed by `autospec-listen` land carrying `needs-classify`. They
are NOT eligible for `/autospec-run` until `/autospec-classify`
transitions them onto the implementation queue. This runbook documents
the manual sweep command, a sample crontab entry that runs it daily,
and common failure modes.

There is **no TTL-based auto-promotion**. Sweeps must be invoked
explicitly — manually or via cron.

## Manual sweep

Run the classifier from a clone of the target repo:

```bash
cd /path/to/repo
claude /autospec-classify
# or, on opencode:
opencode @autospec-classify
# or, on codex CLI:
codex /autospec-classify
```

The classifier walks both `auto-implement` AND `needs-classify` issues,
applies the `ctx:*`/`reasoning:*` rubric, and on `needs-classify`
issues ALSO performs:

```bash
gh issue edit <N> --add-label auto-implement --remove-label needs-classify
```

The skill is idempotent and safe to re-run.

## Sample crontab

A daily 03:00 local-time sweep on a Linux box with `gh` already
authenticated to a GitHub host:

```cron
# /etc/cron.d/autospec-needs-classify-sweep
SHELL=/bin/bash
PATH=/usr/local/bin:/usr/bin:/bin
GH_PAGER=cat

# minute hour dom mon dow user                command
0       3    *   *   *   autospec  cd /srv/autospec/<repo> && claude /autospec-classify >> /var/log/autospec/needs-classify-sweep.log 2>&1
```

The explicit env var preamble (`SHELL`, `PATH`, `GH_PAGER`) is required
because cron starts with a near-empty environment; without `PATH`,
`gh` and `claude` won't resolve. `GH_PAGER=cat` keeps `gh` from trying
to spawn `less` under cron.

Substitute the harness binary (`claude` / `opencode` / `codex`) and the
clone path for the repo you want to sweep. One cron entry per repo.

## Failure modes

- **`gh: not authenticated`.** The cron user has no `gh auth` token.
  Resolve once by `sudo -u autospec gh auth login` (or by setting
  `GH_TOKEN` in the cron preamble for unattended use).
- **Same issue keeps tripping the sweep.** The issue body is missing
  one of the rubric's required sections (`## Files to read first` /
  `## Implementation scope`). The classifier adds a
  `needs-autospec-template` label and skips it instead of looping. Fix
  the issue body or close the issue; the next sweep will then proceed
  normally.
- **`gh project list` denied.** The classifier's optional Phase 3.5
  board-assignment step requires Projects v2 read access. The sweep
  still labels the issues — only board assignment is skipped — so
  this is non-fatal. Grant the relevant scope to the auth token to
  re-enable board assignment.
- **Cron silently does nothing.** Verify `MAILTO` is set on the
  crontab and check the log file for stderr. The sweep should print a
  `Phase 3.5 summary` line on every run, including dry runs that
  found nothing.

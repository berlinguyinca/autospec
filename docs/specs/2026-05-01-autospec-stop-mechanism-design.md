# Autospec Stop Mechanism — Design Spec

**Date**: 2026-05-01
**Repo**: github.com/berlinguyinca/autospec
**Status**: Approved (Phase 2 brainstorm complete; ready to decompose into issues)

## 1. Goals

Two ways to halt a running autospec, both leaving clean state so a later
`/autospec-run` resumes from where it stopped:

1. **Graceful** — finish the current `process(ISSUE)` to its natural end
   (success → admin-merge, or 3-iter failure → label restore + comment).
   The monitor's outer loop exits BEFORE dispatching the next issue.
2. **Immediate** — abort the current `process(ISSUE)` at the next major-step
   boundary; commit any uncommitted work, push the branch, mark the issue
   `paused-by-user` with a `## Resume context` body block, exit.

Per Phase 2 user directive: also add a `/autospec-stop` slash command (6th
skill) PLUS inline `^\s*stop(\s+--\w+)*\s*$` sub-modes in `/autospec` and
`/autospec-run` that map to the same flag-write.

Non-goals (per Phase 2 Q4 fail-safe defaults): force-merge of in-flight
WIP, OS-PID-level signals, parallel multi-issue stop semantics (the
monitor is sequential by contract).

## 2. Architecture

### 2.1 New 6th skill: `/autospec-stop`

Lock-step trio sibling of the existing five autospec-* skills. Wraps the
underlying helper script `scripts/autospec-stop.sh`. Slash invocation
shape:

```
/autospec-stop                  # default --graceful
/autospec-stop --graceful       # finish current issue, exit
/autospec-stop --immediate      # abort at next step boundary, commit+push+mark
/autospec-stop --status         # print flag state + paused-by-user count + last monitor progress
/autospec-stop --resume         # remove paused-by-user labels, delete stop.flag
```

Skill scaffold mirrors the existing autospec-classify pattern:

```
skills/autospec-stop/
  SKILL.md
  README.md
  install.sh
  uninstall.sh
  opencode/agent.md
  codex/prompt.md
```

### 2.2 Inline sub-mode in `/autospec` + `/autospec-run`

Both skills' SKILL.md grow a regex-gated short-circuit (mirroring the
existing `^\s*update\s*$` self-update mode block):

```
If the feature-request argument matches the regex
`^\s*stop(\s+--\w+)*\s*$` (case-insensitive), this skill enters stop
mode and does not run the normal pipeline:
1. Dispatch to `bash scripts/autospec-stop.sh <args>`.
2. Print the helper's stdout to the user.
3. Stop. Do not enter Phase 0 or any pipeline phase.
```

This lets the user type `/autospec stop --immediate` mid-conversation
without invoking a new skill. The new `/autospec-stop` skill remains
the canonical / discoverable entry point.

### 2.3 Sentinel file

`~/.autospec/stop.flag` — single file. Two-line format:

```
graceful
2026-05-01T22:47:13Z wohlgemuth@laptop
```

Line 1: `graceful` or `immediate`. Line 2: ISO-8601 UTC timestamp of
write + `<user>@<hostname>` for audit. Atomic write via temp+`mv`.

### 2.4 Monitor & process(ISSUE) sentinel checks

The autospec monitor outer loop (skills/autospec-run/SKILL.md:163–188
and skills/autospec/SKILL.md:412–424) gets two new check points:

- **Outer loop, before dispatching next issue** — read stop.flag. If
  present (regardless of mode), exit with HARD SHUTDOWN final report.
  This handles `--graceful` cleanly.
- **Inside process(ISSUE), at step boundaries** — between steps 5↔6
  (after push, before PR create), 7↔8 (after LGTM, before merge), 8↔9
  (after merge or before failure cleanup), and at the start of each
  inner-loop iteration. If `~/.autospec/stop.flag` content is
  `immediate`, abort with the §5 abort-clean procedure. If `graceful`,
  finish the current issue normally (the next outer-loop check
  handles exit).

## 3. Operator-facing UX

### 3.1 Helper script `scripts/autospec-stop.sh`

```bash
$ bash scripts/autospec-stop.sh --graceful
[autospec-stop] sentinel written: ~/.autospec/stop.flag = graceful
[autospec-stop] monitor will exit after current issue. 5 issues remaining.

$ bash scripts/autospec-stop.sh --immediate
[autospec-stop] sentinel written: ~/.autospec/stop.flag = immediate
[autospec-stop] current process(ISSUE) will commit + push + mark paused at next step boundary.

$ bash scripts/autospec-stop.sh --status
flag: graceful (set 2m ago by wohlgemuth@laptop)
paused-by-user: 0 issues
last progress: [monitor] iter 14: issue #158 result=merged elapsed=152s

$ bash scripts/autospec-stop.sh --resume
resumed: 2 issues had paused-by-user removed (#207, #209)
flag deleted.
```

### 3.2 Slash invocation paths

```
/autospec-stop --immediate          # canonical (new skill)
/autospec stop --immediate          # inline sub-mode in existing skill
/autospec-run stop --graceful       # inline sub-mode in monitor skill
```

All three paths route through the same `scripts/autospec-stop.sh`.

## 4. Data model

| Path | Format | Producer | Consumer |
|---|---|---|---|
| `~/.autospec/stop.flag` | 2 lines: `<mode>\n<ISO8601> <user>@<host>\n` | `scripts/autospec-stop.sh` | monitor outer loop + process(ISSUE) step checks |
| `paused-by-user` GitHub label | color `#d4c5f9` (lavender), idempotent create | process(ISSUE) abort path on `--immediate` | `gh issue list --label paused-by-user` operator sweep |
| `## Resume context` block in issue body | Markdown with `<!-- autospec-resume:begin --> ... <!-- autospec-resume:end -->` markers | process(ISSUE) abort path | Operator review; future `/autospec-run` resume |

Block format:

```markdown
## Resume context

- **Last step**: <N> (<step name>) completed; aborted before step <N+1>
- **Branch**: feat/<slug> at <sha-short>
- **Diff stat**: <git diff --stat origin/main output>
- **Tests last seen**: <last validate.sh + bats result>

<!-- autospec-resume:begin -->
*Auto-paused by user immediate stop on YYYY-MM-DD.*
<!-- autospec-resume:end -->
```

The block sits before the first `## Dependencies` line (or at end-of-body
if absent), mirroring the existing `## Model fit` and `## Quality lint`
block conventions.

## 5. Failure handling matrix

| Scenario | Outcome |
|---|---|
| Stop flag set DURING a successful merge (between step 8 LGTM and admin-merge call) | Finish the merge; no pause label needed (issue is closed). Exit cleanly. |
| `--graceful` then `--immediate` set in succession | Last write wins; immediate overrides graceful (file content is overwritten). |
| Stop during step 1 (worktree add) | Just abort; no commit/push needed; `gh issue edit --add-label auto-implement --remove-label in-progress-by-bot`. No pause label. |
| Stop during step 2–4 (TDD + commits) | `chore: WIP — autospec stop` commit on any unstaged changes; `git push -u origin <branch>`; mark `paused-by-user`; insert resume block. NO PR yet (step 6 not reached). |
| Stop during step 5–6 (push, PR create) | Commit any unstaged work; push; PR may exist or not. If PR exists, leave it open with a comment `paused by operator`. Mark `paused-by-user`; insert resume block. |
| Stop during step 7 inner loop | Commit any unstaged work; push; leave PR open; mark `paused-by-user`; insert resume block. Do NOT close PR — operator decides on resume. |
| Stop during step 9 (failure cleanup) | Allow cleanup to complete; the failure path already restores `auto-implement`. No pause needed. |
| Stale flag (>24h since timestamp) | Monitor warns to stderr + ignores. Resume cleans the file. Prevents accidental "set then forgot" lockouts. |
| `--resume` with no paused issues | Silent flag deletion; print `no paused-by-user issues; flag deleted.` |
| `--resume` with paused issues | Strip `paused-by-user` from each affected issue; restore `auto-implement`; KEEP the `## Resume context` block as audit trail (don't strip body). Delete flag. |
| `--status` with no flag | Print `no stop signal active`. |
| Two operators (different machines) racing on stop | File mode is single-line atomic write; whichever wins is the active mode. Audit line tracks who wrote. |

## 6. Sentinel check protocol

Bash inserted into the monitor outer loop (canonical reference; lock-step
across the trio):

```bash
# autospec-stop sentinel check — outer loop, top of each iteration
if [ -f "$HOME/.autospec/stop.flag" ]; then
    MODE=$(head -1 "$HOME/.autospec/stop.flag" 2>/dev/null || echo "")
    TIMESTAMP=$(sed -n '2p' "$HOME/.autospec/stop.flag" 2>/dev/null | awk '{print $1}')
    AGE_SECS=$(( $(date -u +%s) - $(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$TIMESTAMP" +%s 2>/dev/null \
        || date -u -d "$TIMESTAMP" +%s 2>/dev/null || echo 0) ))
    if [ "$AGE_SECS" -gt 86400 ]; then
        echo "WARN: stale stop.flag ($AGE_SECS s old); ignoring" >&2
    elif [ "$MODE" = "graceful" ] || [ "$MODE" = "immediate" ]; then
        echo "[monitor] stop signal received: $MODE — exiting"
        # HARD SHUTDOWN with final report
        exit 0
    fi
fi
```

The same check inserted at process(ISSUE) step boundaries (immediate-only):

```bash
# autospec-stop sentinel check — inside process(ISSUE), after each major step
if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
    # commit any unstaged changes, push, mark paused, insert resume block, exit
    bash scripts/autospec-stop.sh --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
    exit 0
fi
```

## 7. Resume semantics

`/autospec-stop --resume` is the canonical path:
1. List `gh issue list --repo <repo> --label paused-by-user --state open`.
2. For each: `gh issue edit <N> --remove-label paused-by-user --add-label auto-implement`. Keep the `## Resume context` block in the body — audit trail.
3. Delete `~/.autospec/stop.flag`.
4. Print summary: `resumed: <K> issues (<list>); flag deleted.`

The next `/autospec-run` (or `/autospec`) invocation picks up the
unblocked `auto-implement` issues normally — no special resume code path
needed; the implementer reads the issue body's `## Resume context` block
and continues from the documented step.

## 8. Lock-step + script contract

`scripts/autospec-stop.sh` interface:

```bash
#!/usr/bin/env bash
# Usage:
#   scripts/autospec-stop.sh [--graceful|--immediate|--status|--resume|--help]
#   scripts/autospec-stop.sh --abort-current-issue <ISSUE_N> <BRANCH> <STEP>   # internal, called by process(ISSUE)
#
# Default: --graceful.
# Exit 0 on success, non-zero on argument error or gh failure.
# All gh calls scope to the cwd's git remote (gh repo view).
```

Pure-bash, `set -eu`, dependencies: `gh`, `git`, `date`, `awk`, `sed`. No `jq`
required for the CLI surface (helper functions can use `--jq` flag of
`gh issue list` directly).

`--abort-current-issue` is internal — the process(ISSUE) sentinel check
calls it to perform the WIP-commit + push + label-swap + body-block
insertion. Not exposed in `--help`.

## 9. Testing

Per AGENTS.md (validation in lieu of code tests; real services).

### 9.1 Unit (bats)

`tests/unit/test_autospec_stop.bats` — exercise CLI against sandboxed
`$HOME/.autospec/`:
- `--graceful` writes flag with mode=graceful + timestamp.
- `--immediate` writes flag with mode=immediate.
- `--status` with no flag → "no stop signal active".
- `--status` with flag → reports mode + age.
- `--resume` deletes flag + reports.
- `--resume` with mock paused issue (gh-stubbed) reports it stripped.
- Stale flag (24h+) → status warns.
- `--graceful` then `--immediate` → flag content is `immediate` (last-write-wins).

### 9.2 Sentinel-poll integration (bats)

`tests/unit/test_stop_sentinel_check.bats` — extracts the canonical bash
check from `skills/autospec/SKILL.md` Phase 4 section via awk, sources
into a sandboxed shell, asserts:
- No flag → no exit, continue.
- Flag with mode=graceful → exit 0 with "stop signal received: graceful".
- Flag with mode=immediate → exit 0 with "stop signal received: immediate".
- Flag with stale timestamp → WARN + continue.
- Flag with malformed mode → WARN + continue.

### 9.3 Inline sub-mode regex (bats)

`tests/unit/test_stop_inline_subagent_regex.bats` — asserts the regex
`^\s*stop(\s+--\w+)*\s*$` matches:
- `stop`, `stop --graceful`, `stop --immediate`, `stop --status`, `stop --resume`,
- `  stop  --graceful  ` (with whitespace).

And rejects:
- `stop me now`, `stopover`, `stop ; rm -rf`, `stop\n--graceful`,
  `STOP HERE`, `stoppppp`.

### 9.4 Validator extension

`autospec validate` extensions:
- `check_required_files` adds `autospec-stop` to the list (so the new
  skill's full scaffold presence is enforced).
- New `check_stop_mode_section` asserts every multi-harness skill's
  trio carries `## Stop mode` heading (parallels existing
  `check_self_update`).

No e2e gh test — sentinel mechanics are pure-bash file IO; integration is
the dispatch shape, which the lock-step + grep checks already cover.

## 10. Documentation

- `AGENTS.md` — new `## Stop mode authority` heading parallel to `##
  Auto-merge authority`. Documents: graceful vs immediate semantics,
  WIP-commit-on-immediate-stop contract, paused-by-user label, resume
  procedure.
- `README.md` — short `## Stopping a run` paragraph + invocation
  examples.
- `docs/runbook.md` (if exists; skip otherwise) — operator sweep:
  `gh issue list --label paused-by-user` + `--resume` flow.

## 11. Decomposition outline

EPIC umbrella: **Add stop mechanism (graceful + immediate) to autospec**.

| # | Title | Files | Deps |
|---|---|---|---|
| 1 | feat(scripts): add `scripts/autospec-stop.sh` (CLI: --graceful/--immediate/--status/--resume + internal --abort-current-issue) | 1 | — |
| 2 | feat(autospec-stop): add skill trio (SKILL.md + opencode/agent.md + codex/prompt.md) wrapping helper | 3 | 1 |
| 3 | feat(autospec-stop): add install.sh + uninstall.sh + README.md | 3 | 2 |
| 4 | feat(install): extend top-level install.sh + uninstall.sh to include autospec-stop | 2 | 3 |
| 5 | feat(autospec): wire monitor outer-loop sentinel poll + process(ISSUE) inter-step checks (lock-step trio) | 3 | 1 |
| 6 | feat(autospec-run): wire monitor outer-loop sentinel poll + process(ISSUE) inter-step checks (lock-step trio) | 3 | 5 |
| 7 | feat(autospec): add inline `stop` sub-mode regex shortcut (lock-step trio) | 3 | 5 |
| 8 | feat(autospec-run): add inline `stop` sub-mode regex shortcut (lock-step trio) | 3 | 6 |
| 9 | feat(validate): extend with `check_stop_mode_section` + `check_required_files` for autospec-stop | 1 | 7, 8 |
| 10 | test(unit): add `tests/unit/test_autospec_stop.bats` for CLI flags | 1 | 1 |
| 11 | test(unit): add `tests/unit/test_stop_sentinel_check.bats` extracting + running poll-point bash | 1 | 5 |
| 12 | test(unit): add `tests/unit/test_stop_inline_subagent_regex.bats` for sub-mode regex | 1 | 7 |
| 13 | docs: add `## Stop mode authority` to AGENTS.md + README mention | 2 | 5 |

Total: 13 child issues + 1 umbrella.

## 12. Out of scope

- Force-merge of in-flight WIP on graceful stop.
- OS-PID-level signal handling (kill -USR1 etc).
- Parallel multi-issue stop semantics (monitor is sequential).
- Auto-resume on file delete (operator must invoke `--resume` explicitly so the WIP-commit audit trail is reviewed first).
- Stop signals for Phase 1 / Phase 3 / Phase 3.5 subagents (those are short-lived; restart the orchestrator to abort).
- Telemetry on stop-resume cycle frequency.

## 13. Open questions

None at spec time. All five Phase-2 questions resolved.

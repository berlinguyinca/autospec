
# autospec-resume (harness-neutral)

Detect an interrupted autospec run from durable run-state + heartbeats on a
*fresh* start and auto-continue it. All decision logic routes through
`bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/resume-scan.sh" "$@"`.

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-resume -->

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-resume/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-resume.md`
   - Codex CLI:   `~/.codex/prompts/autospec-resume.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not enter any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-resume found; run install.sh first.` and exit.

## Harness detection

This skill is a thin dispatch wrapper — the relaunch happens through the durably-captured command, so it runs at the implementation tier. Detect your harness and pick the matching subagent tier:

- **TIER_A** (architecture / deep reasoning): not used here; this skill never plans.
- **TIER_B** (implementation work): the default — run the scan helper, print its one-line decision.

If your harness lacks a subagent primitive, run the helper inline and **silently** fall back to the in-context path rather than erroring. Never escalate a missing capability into a hard failure.

## Stop mode

If the feature-request argument matches the regex `^\s*stop(\s+--\w+)*\s*$`
(case-insensitive), defer to `/autospec-stop`: dispatch to
`bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" <args>`,
print its stdout, and do not enter any resume pipeline. Resume never halts a
running monitor — that is `/autospec-stop`'s job.

## Invocation

```
/autospec-resume [--resume-partial] [--repo <owner/name>] [--dry-run]
```

Dispatches directly to `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/resume-scan.sh" "$@"`. All decision logic lives in the helper script.

| Flag | Behaviour |
|---|---|
| (none) | Scan `in-progress-by-bot` and open `auto-implement` issues, apply the auto-resume pre-conditions, recover only exact stale-startup claims through Rust, and if all pass relaunch via the durably-captured command (clean-restart off `origin/main`). |
| `--resume-partial` | Additionally re-attach `/tmp/wt-<branch>` **only when** `heartbeat.host == $(hostname)`; cross-host or missing host silently clean-restarts off `origin/main`. |
| `--repo <owner/name>` | Target a specific repo (used by the boot supervisor, which iterates the registry). |
| `--dry-run` | Print the recovery and relaunch actions that *would* run; invoke neither action; change nothing; exit 0. |

**Exit codes:** `0` = nothing to resume / `--dry-run` / resumed (one-line reason printed); `1` = hard error (bad args, no `gh`).

## Auto-resume contract (enforced by the helper)

- **No second lock / idempotency.** Resume relaunches the *run*; the relaunched monitor reads the existing GitHub CAS lock-comment through `autospec claim state read` (lowest-comment-id wins, loser self-cleans). Resume itself never writes a run-state comment and never adds any new lock. Two concurrent resumes (supervisor + human, or two hosts) converge to exactly one claim per issue at the existing CAS boundary.
- **Pre-conditions (ALL required, else exit 0, no relaunch):** (a) ≥1 crashed `in-progress-by-bot` issue or exact stale-startup candidate from open `auto-implement`; (b) no `~/.autospec/stop.flag`; (c) no issue labeled `paused-by-user`; (d) not all issues closed.
- **Stranded startup recovery.** Scan both relevant labels, but accept `auto-implement` only when Rust run-state is the exact stale `claimed` / `heartbeat-pending:none` sentinel for the requested repository and issue. Delegate every ownership mutation to `autospec claim state recover-stale-startup`; relaunch only when Rust returns an exact matching `recovered: true` result.
- **Crash-vs-live.** Treat an issue as crashed (eligible) only when its age, computed from run-state **server** `updated_at` (never a local clock — mirrors `autospec-watchdog.sh:386-405`), satisfies `step=claimed && age>=300` OR `age>=10800`. Otherwise assume a live/slow worker elsewhere and do **not** steal.
- **Cross-host.** `--resume-partial` re-attaches `/tmp/wt-<branch>` only when `heartbeat.host == $(hostname)`; cross-host or missing `host` MUST clean-restart off `origin/main` (the crashed worktree is on a dead machine's local `/tmp`).
- **Durable relaunch command.** The command comes from the registry `~/.autospec/active-runs/<repo-slug>.json` written by `/autospec-run` at monitor launch — never an attacker-supplied path. After a reboot (env cleared) the registry alone yields a runnable command.
- **Attempt cap.** Capped at `AUTOSPEC_RESUME_MAX_ATTEMPTS` (default 3) consecutive attempts without forward progress. At the cap, halt, print the cap-reached reason, and surface. The counter resets on forward progress (any issue `merged`).

## Required capabilities & harness adapter

This workflow assumes a small set of capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Run shell command            | `Bash`                               | `bash` tool                              | `shell` / `apply_patch`                  | Ask user to run manually                           |
| Ask the user a question      | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Subagent model tier          | Tier B: `sonnet` + medium thinking   | Tier B: smaller-tier `task` + medium reasoning | Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Fall back UP on unavailability |
<!-- autospec-block:harness-adapter-core -->

**Model tier:** Tier B (implementation work) — this skill is a thin dispatch wrapper.

## Procedure

1. Run startup self-update block above.
2. Parse the user's invocation arguments.
3. Execute: `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/resume-scan.sh" "$@"`
4. Print the helper's one-line decision to the user and exit.

If `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/resume-scan.sh` is not found, print:
```
autospec-resume: helper not found at ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/resume-scan.sh.
Reinstall autospec-resume from the autospec repo, then retry.
```
and exit non-zero.

## Hard rules

- Never write a run-state comment from this skill and never add a new lock. The relaunched monitor claims through the existing GitHub CAS lock only.
- Never invoke stale-startup recovery or the relaunch command under `--dry-run`.
- Never re-attach a cross-host or missing-host worktree under `--resume-partial`; clean-restart off `origin/main`.
- Never exceed `AUTOSPEC_RESUME_MAX_ATTEMPTS` consecutive auto-resume attempts; halt and surface at the cap.
- This skill is a thin wrapper. All behaviour is in the helper script.

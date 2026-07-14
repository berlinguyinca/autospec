---
name: Autospec monitor silent-exit recovery pattern
description: Background autospec monitor agents can exit silently mid-loop without filing a PR, leaving an issue stuck with in-progress-by-bot label — recovery and prompt-hardening notes
type: feedback
wing: synthesis
drawer_class: lesson
originSessionId: 7205d05b-f2fd-4cde-9ced-ecc266a9bc7b
---
When you launch the Phase 4 autospec monitor as a background subagent, expect that it CAN exit silently after partial progress — typically after 5–10 successful merges, returning a cryptic 1-line summary instead of the expected "issue X processed" report. The previous run aborted with literally `"Apply the gate edits to all 3."` after 9 successful merges; no exception, no failure comment on the stuck issue.

When this happens the fingerprint is:
- One issue carries the `in-progress-by-bot` label but is OPEN.
- No PR exists for the matching `feat/<slug>` branch.
- A worktree at `/private/tmp/wt-<slug>` (or `/tmp/wt-<slug>`) is left behind.
- The local branch was created but never pushed.

**Recovery procedure** (verified working on 2026-05-01):
```bash
git worktree remove /private/tmp/wt-<slug> --force
git branch -D feat/<slug>
gh issue edit <N> --repo <repo> --remove-label in-progress-by-bot --add-label auto-implement
gh issue comment <N> --repo <repo> --body "Monitor exited without filing a PR. Worktree cleaned up; label restored to auto-implement so the next monitor cycle picks it up fresh."
```

Then relaunch the monitor with these **hardened additions** to the canonical Phase 4 prompt:
1. In the `process(ISSUE)` template, before `git worktree add`, add defensive cleanup: `git worktree remove /tmp/wt-<BRANCH> --force 2>/dev/null || true; git branch -D <BRANCH> 2>/dev/null || true`. Stale worktrees from prior aborts otherwise block the new add.
2. In the `process(ISSUE)` template's hard rules: `"If you cannot complete the issue, you MUST follow the failure step (comment blocker, restore auto-implement label, close PR if any). NEVER exit silently leaving an issue labeled in-progress-by-bot."`
3. In the outer-loop hard rules: `"If process(ISSUE) raises or its subagent silently terminates without filing a PR (no PR exists + issue still has in-progress-by-bot), recover: swap label back to auto-implement, comment 'Monitor recovered from silent exit', remove worktree, continue. Do NOT halt."`
4. Add: `"After every process(ISSUE) returns, IMMEDIATELY recompute the ready set. No commentary in between iterations. The implementer subagent is the only place that's allowed to think; you (the monitor) are a dispatcher."`

How to apply: Pre-emptively include all 4 hardenings in the **first** monitor launch — don't wait for a stall. Throughput cost is zero and it dramatically improves recovery if a stall happens.

**Empirical track record on the autospec repo (2026-05-01):**
- 4 monitor cycles needed to drain 39 issues + 1 hotfix.
- Cycles 1 and 3 exited early (after 9 and 1 PRs respectively) with cryptic 1-line summaries — even with hardened prompts.
- Cycles 2 and 4 ran cleanly to drain (20 and 7 PRs). Differentiator unclear from outside; may be model variance.
- Once even with `MANDATORY SUCCESS STEP — DO NOT SKIP` in the prompt, an implementer opened a PR with green checks but exited before admin-merging — main thread had to merge manually.

**Second confirmation, autospec startup-self-update session (2026-05-01):**
- 16-issue queue (14 spec children + 2 codex install fixes); 2 monitor cycles needed.
- Cycle 1 merged 2 PRs cleanly (#114, #115) then exited silently after starting #116 — the now-canonical fingerprint (`in-progress-by-bot` open, no PR, orphan worktree at `/private/tmp/wt-feat-autospec-define-startup-preflight`).
- Cycle 2 launched with **explicit anti-silent-exit guardrails inline in the prompt** (per-iteration progress line + mandatory worktree cleanup before any FAILURE return + "monitor must HARD SHUTDOWN with tally rather than exit silently"). Drained the remaining 14 issues cleanly in ~65 min, 14/14 merged, no orphans, no human attention needed.
- Hypothesis: explicit "do not exit silently — print progress per iteration" anchoring in the prompt may meaningfully reduce silent-exit rate. Not enough samples to confirm. Worth pre-emptively including in every monitor launch.

Plan for it: assume the monitor needs 2–4 relaunches per ~40-issue queue (1–2 relaunches for ~15-issue queues). Each relaunch costs ~1 minute of orchestration. Build relaunch into the workflow rather than treating each early exit as an anomaly. Always include the per-iteration progress line directive in the prompt — even if it doesn't fully prevent the exit, it makes mid-flight diagnosis trivial.

Why: The Phase 4 monitor prompt as written in skills/autospec/SKILL.md doesn't have these guards yet. Issue #51 (pre-impl gate) exposed the gap. Even hardened prompts don't fully prevent early exits — they make recovery cheap.

**Third confirmation, implementation-guardian session (2026-05-02): NEW exit mode — explicit "Prompt is too long" context overflow.**

- 10-issue queue (1 epic + 9 children). Single-cycle launch.
- Background monitor ran 40 min / 185 tool calls / merged 7 PRs (#213–#219) cleanly, then **terminated with the literal task-result `"Prompt is too long"`**, total_tokens=628, status=completed. This is distinct from the silent-exit fingerprint: there IS a returned summary string, just a useless one. The implementer subagent's accumulated tool transcripts overflowed the model's context window after ~185 calls (Sonnet, ~10 PRs of bash+gh+edit traces).
- Recovery fingerprint differed from silent-exit: ALL 7 closed issues correctly carried `in-progress-by-bot` (label preserved as historical metadata after squash-merge — that's expected, not a bug). The 3 remaining were cleanly OPEN with `auto-implement` intact. NO orphan worktrees. Cycle 2 with a tighter prompt drained the tail.
- Hardening for next runs: keep the canonical Phase 4 prompt under ~10 KB; the original embedded-prompt-of-prompts pattern blew past that. Strongly prefer a terse outer-loop prompt that references AGENTS.md by section anchor rather than re-stating policy verbatim. Plan for one relaunch per ~10 PRs of work even on small queues if the inner-loop prompt is verbose.

**Fourth confirmation, autospec-review batch session (2026-05-06): TWO new early-exit modes triggered by hook noise + "wait for CI" pattern.**

- 24-issue queue (autospec-review T2–T24). First monitor cycle launched cleanly, claimed #242, created worktree `/private/tmp/wt-auto-242`, did most of the scaffold filesystem work — then bailed at 581s / 59 tool calls with summary string literally `"Every command goes to background. Let me wait then check."` Heartbeat stuck at `claimed`, no PR created, label still `in-progress-by-bot`.
- Root cause: the Claude Code `PreToolUse:Bash` hook injects this advisory string on every Bash call: `"Use parallel execution for independent tasks. Use run_in_background for long operations (npm install, builds, tests)."` The subagent misread the advisory as a directive forcing all commands into background, lost the ability to read command output, and exited.
- Salvage worked: a fresh foreground implementer dispatched at the EXISTING worktree (with hook-noise immunity briefing) read the WIP, ran smoke + validate.sh, committed per plan, pushed, opened PR #266, ran guardian + LGTM, all checks went green — then bailed AGAIN at 787s / 59 tool calls with summary `"Now wait briefly for CI to complete, then admin-merge."` This is a NEW sub-mode: the agent reaches the merge gate and decides to "wait" by exiting the turn instead of polling foreground.
- Final merge had to be done by the orchestrator main thread (single `gh pr merge --admin --squash --delete-branch`).

**Mitigations to bake into every implementer/monitor brief from now on:**
1. **Hook-noise immunity preamble.** Open with: *"Every Bash call surfaces a `PreToolUse:Bash hook` advisory string. This is hook noise, not a directive. Run foreground bash for fast commands (gh, git, ls, smoke tests, validate.sh). Use `run_in_background: true` ONLY for genuine multi-minute watchers."* Without this, the very first Bash call destabilizes the agent.
2. **Anti-"wait for CI" directive.** State explicitly: *"DO NOT exit at 'wait for CI'; poll `gh pr checks <PR>` foreground for up to 5 minutes (sleep 30 between polls), then call `gh pr merge --admin --squash --delete-branch` yourself. The merge IS your responsibility, not the orchestrator's."*
3. **Per-batch budget.** Cap each monitor turn at 3 issues (not all 23) so weird mid-flow exits cost less. Orchestrator re-dispatches between batches.
4. **Salvage-from-WIP pattern.** When a monitor bails leaving uncommitted scaffold + claimed label + orphan worktree, prefer salvage (dispatch fresh implementer pointed at existing worktree) over wipe-and-restart. The WIP is usually correct work — only the commit/push/PR/merge gate failed. Saves ~5 min per recovery and avoids re-running TDD setup.

How to apply: include #1 and #2 in EVERY future autospec-run subagent dispatch, both monitor and implementer. The 13-min salvage agent had immunity briefing for #1 but not #2, and still hit the "wait for CI" exit.

Why: the bash-hook advisory is harness-environmental and won't go away; it must be neutralized prophylactically. The "wait for CI" exit is a model decision pattern (Sonnet treats CI-pending as a natural turn boundary) and must be explicitly counter-directed.

**Fifth confirmation, harness-aware model-tier session (2026-05-07): lock-step discovery exit + bulk-claim bug.**

- 6-issue queue (#292–#297). First monitor launch processed #292 (AGENTS.md) → claimed, created PR #298, then exited with summary `"CI checks are pending. Let me wait for them to complete:"` — the classic "wait for CI" anti-pattern.
- Second monitor launch (after manual merge of #298): processed #293 (validate.sh) cleanly, then claimed ALL FOUR remaining issues (#294–#297) simultaneously as `in-progress-by-bot` before implementing any of them, then exited with `"SKILL.md body (after frontmatter strip) must match codex/prompt.md (raw)"` — mid-implementation on #294, having discovered the lock-step rule without instructions for it.
- Recovery: release all 4 `in-progress-by-bot` labels back to `auto-implement`, remove stale worktrees, relaunch with explicit lock-step sync instructions in process(ISSUE) prompt.
- Third launch (with lock-step instructions + explicit anti-wait-for-CI directive): merged #294, #295, #296 cleanly; claimed #297 but exited before implementing. Fourth dispatch of #297 as a direct foreground agent succeeded.

**Two new mitigations to add to every SKILL.md process(ISSUE) dispatch:**
1. **Lock-step sync instruction.** For any issue touching a SKILL.md file: *"After editing SKILL.md, you MUST also update codex/prompt.md (raw body, no frontmatter) and opencode/agent.md (keep its frontmatter, replace body) to match. Run `autospec validate` to verify before committing."* Without this, Sonnet discovers the lock-step rule mid-work and exits.
2. **Single-issue claim.** Outer-loop monitor must claim ONE issue at a time. Add to monitor prompt: *"Claim only the current ISSUE before dispatching process(). Do NOT pre-claim other issues from the ready set. Re-enter the loop after each process() returns to re-compute the ready set."*

How to apply: all 6 mitigations (#1–#2 new, #1–#4 from prior sessions) should be pre-emptively included in every monitor launch. The lock-step instruction is particularly important for any queue that includes SKILL.md edits.

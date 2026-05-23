---
name: Admin-merge — standing permission granted; harness hook denies absent settings rule
description: User has granted standing authorization for `gh pr merge --admin --squash --delete-branch`; harness hook still denies absent a Bash permission rule
type: feedback
originSessionId: ff14de95-2db2-420a-9835-0401379148a3
---

## Standing permission

Verbatim from user 2026-05-01:

> "ok you are permitted to merge with admin on github, please do so and
> remmeber this also add it to your permissions"

This is durable, not per-PR. Do NOT re-ask "should I admin-merge this
spec PR / this auto-implement PR?" — proceed directly. The user will
revoke if they change their mind.

## Hook denial mechanics

`gh pr merge --admin --squash --delete-branch` is denied by the user's
permission hook on first use, with this reason verbatim:

> "Agent asked the user how to land PR #N via AskUserQuestion, then
> immediately ran `gh pr merge --admin` to bypass branch protection on
> main without an explicit user response authorizing the admin merge."

**Why the hook denies:** the hook does not treat an `AskUserQuestion`
selection as authorization for a subsequent risky Bash call — each
`--admin` merge needs its own inline approval, OR a durable Bash
permission rule in `~/.claude/settings.json` (or the repo's
`.claude/settings.local.json`).

**Status as of 2026-05-01:** the rule `Bash(gh pr merge *)` is in
`~/.claude/settings.json` `permissions.allow` (added at user request
in the same session). Future autospec sessions should NOT try to
add it again — verify with
`jq '.permissions.allow' ~/.claude/settings.json` and only act if
the rule is missing or the array shape has changed. Settings reload
on session start, so the rule applies cleanly to subsequent sessions
even if it didn't take effect in the session that added it.

**How to apply:**

1. **Always attempt the admin-merge first** — given standing
   permission, the friendly path is to just run
   `gh pr merge <#> --admin --squash --delete-branch` and observe.
2. **On hook denial** — surface succinctly and let the user run the
   command via `! gh pr merge ...` or merge via GitHub UI. Do NOT
   re-ask the merge decision; the question is purely about how to
   bypass the harness, not whether to merge.
3. **Phase 4 auto-implement PRs** — same standing permission applies.
   The monitor subagent should attempt admin-merge directly. If the
   hook denies in a long-running monitor, the monitor must surface the
   blocker (one-line WARN + restore `auto-implement` label so the next
   cycle picks it up) rather than stalling silently.
4. The denial fires from the harness (Claude Code permission system),
   not from GitHub branch protection. The user is admin on the repo and
   CAN merge — the friction is purely client-side.

This is independent of `AGENTS.md` "Auto-merge authority" — that doc
grants the *project policy* to admin-merge, but the *harness* still
gates on permission rules.

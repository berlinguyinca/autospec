# Autospec Autonomy Charter

**Status:** active default for this operator (`~/.autospec/autonomous.flag` present).
**Boundary:** aggressive (see §3).
**Origin:** derived 2026-06-25 from mining ~2,963 Claude + ~1,538 Codex session
transcripts. See `docs/memory/project_babysit_tax_autonomy_charter.md`.

## 1. Why this exists

Session mining found that the operator's short steering turns (5.2% of all
Claude human turns, 13.4% of Codex turns) are almost never course corrections —
they are **rubber-stamps of a recommendation the agent had already made**. Every
recurring gate has the same shape: the agent states its own next step, then asks
permission, and the operator grants it ("looks good", "ok fix that one once and
for all", "review the whole project for gaps").

The governing rule of this charter:

> **Recommendation = action.** If the agent is confident enough to *recommend* a
> next step, it is confident enough to *take* it and report the result. Asking
> permission for your own recommendation is friction, not safety.

## 2. The four collapsed stop-points

| # | Old behavior (asked) | New behavior (proceed + report) |
|---|---|---|
| 1 | **Design ratification** before writing the spec — "State machine make sense? Test plan look sound? UX feel right?" | Write the spec immediately. Record judgment calls inline as `> AUTONOMOUS ASSUMPTION:` blockquotes (already implemented in `autospec-define` autonomous mode). A self-review pass replaces the human ratify. |
| 2 | **Spec → plan → implement handoff gates** — "review the spec before I invoke writing-plans" / `[run / defer / refine]` | Auto-advance the chain. `autospec-define` defaults to `run` and hands off to `autospec-run` without a gate. |
| 3 | **"Want me to commit / file these gap issues / run fixture-gen?"** for reversible local actions | Just do it and report. No permission turn for reversible, in-scope, local work. |
| 4 | **Queue-drain stall** — "queue empty, next work: none, standing by" | Auto-chain into the next improvement loop: `autospec-review` (already auto-fires post-batch) and, when enabled, `autospec-explore`. The operator's reflexive "review the whole project for gaps" *is* that loop. |

Plus the async-wait pattern: during CI / monitor waits the agent must **push a
notification on each state transition** rather than going silent and waiting to
be pinged "how do they look?".

## 3. The boundary — when autospec STILL pauses (aggressive)

Default-proceed on everything reversible **and** on remote merges/PRs (the
operator already admin-auto-merges). Surface a confirmation ONLY for:

- **Irreversible destructive remote actions** — repo delete/archive, release
  delete, mass label changes, prod DB writes, `rm -rf /`, `DROP`/`TRUNCATE`.
- **Force-push to a protected branch** (e.g. `main`).
- **Out-of-scope file changes** — planned files extending beyond the spec's
  Goal + Implementation outline.
- **Cost over threshold** — token estimate above the aggressive cap (§4).
- **A genuine no-clear-winner fork** — two+ options with no defensible default.
  (Note: a fork where one option is clearly better is *not* this — pick it.)

Enforcement is unchanged and already implemented:
`scripts/autospec-autonomy-gate.sh --check all` is invoked before each
would-have-asked decision; exit 1 surfaces the confirmation even in autonomous
mode. This charter does not weaken the gate — it makes "proceed" the default for
everything the gate does **not** flag.

## Native autonomous runtime support

Linux autonomous execution proves process ownership with pidfds and subreaper
containment. macOS proves the exact kernel boot and process start identities and
isolates executor descendants in an owned process group. Unsupported platforms
fail before creating claims or accountability epics. Every autonomous
`--dry-run`, including start, restart, and resume, is a read-only preview.

## 4. Configuration

| Lever | Default | Aggressive (recommended) |
|---|---|---|
| `~/.autospec/autonomous.flag` | absent (interactive) | **present** |
| `AUTOSPEC_AUTONOMOUS_TOKEN_CAP` | 500000 | **2000000** |
| `~/.autospec/no-review.flag` | absent (review auto-fires) | absent |

Autonomous specs have no per-spec issue-count cap. The separate cumulative
`AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES` resource budget remains in force for the
long-running autonomous conductor.

To raise the token cap persistently, export in your shell profile:

```sh
export AUTOSPEC_AUTONOMOUS_TOKEN_CAP=2000000
```

Turn the whole charter off at any time: `rm ~/.autospec/autonomous.flag`.

## 5. Already built vs. still to build

**Already in place** — `autospec-run` is fully autonomous (no operator gates in
normal operation); `autospec-define` autonomous mode collapses the brainstorm
and the pre-impl gate; the autonomy gate enforces the boundary; `autospec-review`
and `autospec-secaudit` auto-fire after each run batch.

**Still to build** (tracked as autospec meta-improvements):

1. **Queue-drain → `autospec-explore` auto-chain** — on `ALL_DONE`, optionally
   launch the perpetual improvement loop instead of stopping (opt-in via a
   `~/.autospec/explore-on-drain.flag`).
2. **Async-transition push notifications** — emit a notification on each CI /
   monitor state transition so the operator never has to ask "how do they look?".

# Autonomous session-follow design

Issue: [#2566](https://github.com/berlinguyinca/autospec/issues/2566)

## Goal

Let an operator start autonomous Autospec from Codex or Claude and receive live progress
in that same session without making the conductor depend on the session remaining open.

## Decision

The Rust CLI gains `autospec autonomous start --follow`. The command preserves the
existing detached lifecycle: it starts the scoped conductor, monitor, and supervisor,
then keeps only the calling process attached as a log follower. If the scoped conductor
is already running, `start --follow` attaches to it without restarting it or treating
the held lifecycle lease as an error.

Raw `autospec autonomous start` remains detached for compatibility with scripts and
service launchers. Direct multi-harness skill invocation changes:

- `$autospec-autonomous` in Codex follows by default.
- `/autospec-autonomous` in Claude follows by default.
- The OpenCode adapter follows by default to keep the lock-step skill contract.
- An explicit `--detach` preserves non-streaming detached behavior.
- An explicit `--foreground` runs the conductor in the caller as it does today.

The skill adapters add `--follow` only when the operator did not provide
`--follow`, `--detach`, or `--foreground`. This makes the interactive default
transparent while preserving explicit operator intent.

## Public command contract

`--follow` is valid for `start`. It is mutually exclusive with `--detach` and
`--foreground`. `--detach` is valid for `start` and expresses the existing default
explicitly. Supplying either flag to an unrelated subcommand is a usage error.

The Rust help output lists all three launch modes and their lifecycle semantics:

| Mode | Conductor ownership | Current-session output |
| --- | --- | --- |
| default / `--detach` | detached and supervised | start summary only |
| `--follow` | detached and supervised | continuous scoped progress |
| `--foreground` | caller-owned | direct conductor output |

`start --follow --dry-run` prints the detached launch plan and states that the caller
would follow the scoped conductor log; it does not start or attach to a process.

## Start-or-attach behavior

The CLI resolves the repository scope before deciding whether to start or attach.
When no live conductor owns that scope, it performs the existing atomic detached start
and then follows the newly recorded conductor log. When a live conductor already owns
the scope, it skips all start mutations and follows the recorded live unit. Ambiguous,
foreign, or malformed process metadata still fails closed rather than attaching.

This start-or-attach behavior applies only to `start --follow`. Plain `start` retains
its existing lease-conflict behavior so automation can continue detecting duplicate
launch attempts.

The follower is scoped by repository rather than by one immutable logfile. It polls
the scoped unit metadata while streaming. If the supervisor replaces the conductor and
records a new logfile, the follower reports the transition and continues from the new
file. It emits explicit session lines when the conductor stops, fails, is repaired, or
switches logs; discovery and implementation events continue to come from the conductor
timeline. It does not invoke desktop notifications.

## Interrupt and failure semantics

The conductor, monitor, and supervisor are detached before following begins. Therefore
closing the terminal, ending the AI tool call, or pressing `Ctrl-C` affects only the
follower process. It never writes a stop sentinel or signals a lifecycle unit.

Startup errors remain startup errors and do not enter follow mode. An unreadable log or
ambiguous unit record exits nonzero with a scoped diagnostic. A temporary absence during
a supervisor restart is reported and retried; an explicit terminal stop is reported
before the follower exits. Operators use the existing autonomous stop command when they
intend to halt work.

## Skill adapter behavior

The canonical `skills/autospec-autonomous/SKILL.md` body instructs an interactive
harness to execute the installed development-resolved `autospec` command with
`autonomous start --follow --repo-dir "$PWD"` when invoked without another operator
subcommand or launch mode. The generated Claude, Codex, and OpenCode adapters retain
identical bodies under the repository lock-step rule.

The harness must keep the command/tool call active while forwarding progress to the
initiating session. It must not replace the command with a desktop notifier, a detached
agent, or a separate terminal. Explicit commands such as `status`, `stop`, `timeline`,
and `watch` keep their current routing.

## Testing

Rust integration tests first prove:

1. `start --follow` starts detached units and follows the recorded scoped log.
2. A second `start --follow` attaches without restarting the live conductor.
3. Follower termination leaves the recorded conductor PID alive.
4. `--detach` retains detached start output and returns immediately.
5. launch-mode conflicts and unrelated-subcommand usage fail before mutation.
6. `--help` documents `--follow`, `--detach`, and `--foreground`.
7. dry-run output describes follow mode without creating state.
8. log replacement after supervisor repair remains visible in the same follower.

Skill validation proves the interactive default and explicit-mode precedence in the
canonical body, then derives and checks the Claude, Codex, and OpenCode adapters. The
repository validation suite must pass without weakening existing lifecycle or lock-step
checks.

## Out of scope

This change does not alter discovery priorities, issue implementation policy, budgets,
stop sentinels, desktop notification configuration, or the raw CLI detached default. It
does not make Codex or Claude own the conductor process. A structured event transport
beyond the existing scoped logs and timeline remains separate work.

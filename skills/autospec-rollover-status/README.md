# autospec-rollover-status

Read-only diagnostic for the **autospec** auto-context-rollover monitor. Reports
the active session's current context-window usage and its last rollover events,
so you can tell how close a long run is to a `/compact` or handoff.

This is the inspection companion to autospec's [auto context rollover](../../README.md#auto-context-rollover-opt-in)
feature, which injects `/compact` at 50% context and a handoff → clear → resume
at 80%.

## Install

Ships with the autospec suite (Claude Code). Get the whole suite via the one-line
bootstrap:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash
```

Already installed? Re-run the suite installer from the **repository root** with
`--update`, which reinstalls every skill in place:

```bash
./install.sh --update
```

This skill also has its own installer, for updating it alone from **this directory**:

```bash
./install.sh --harness claude --update
```

## Invocation

```text
/autospec-rollover-status
```

It runs `show.sh`, which reads the newest log under `~/.autospec/monitors/` and
prints the last few usage/rollover/compact/handoff events plus the current
context percentage, for example:

```text
Monitor log: ~/.autospec/monitors/<session>.log
---
  [2026-06-16T10:02Z] usage: 48.3% (96600/200000 tokens)
  [2026-06-16T10:14Z] compact: 51.0% (102000/200000 tokens)
---
Current context: 51.0% used
```

When no monitor is active it says so and exits cleanly.

## Herdr-managed panes

Herdr has its own automatic handoff controller, and two policies must never act on
one session. The monitor therefore tests the environment of the process that starts
it (`autospec-session` run from a Herdr pane): a non-empty `HERDR_PANE_ID` means it
logs `{"event": "herdr_managed"}` and exits 0 before touching tmux, so there is no
context percentage to report, and `autospec-session` prints a line to stderr saying so.

- A monitor started from outside Herdr (cron, a daemon, a shell without the
  variable) for a session that is displayed in a Herdr pane is **not** skipped.
- A monitor started from a Herdr pane is skipped even if Herdr's controller does not
  manage what runs inside tmux; that session then has no automatic policy. Set
  `AUTOSPEC_CONTEXT_MONITOR_SKIP_HERDR=0` to keep the monitor.
- When the Herdr controller is switched off for a pane, the monitor stays off there
  too, so that pane has no handoff policy at all.
- Hook mode is skipped the same way (log: `hook-<event>.log`), so the hook's desktop
  notifications are also silent in Herdr panes.
- `no-auto-rollover.flag` is unchanged. `python -m autospec_context_monitor.doctor`
  shows the setting and whether the current environment would be skipped.

Open point: whether Herdr's controller recognises a harness running inside tmux has
not been verified.

## Hard rules

Read-only. The skill never writes files, labels, GitHub state, or log/PID files,
and tolerates a missing log gracefully (exit 0 with a clear message).

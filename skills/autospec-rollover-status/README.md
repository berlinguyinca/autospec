# autospec-rollover-status

Read-only diagnostic for the **autospec** auto-context-rollover monitor. Reports
the active session's current context-window usage and its last rollover events,
so you can tell how close a long run is to a `/compact` or handoff.

This is the inspection companion to autospec's [auto context rollover](../../README.md#auto-context-rollover-opt-in)
feature, which injects `/compact` at 50% context and a handoff → clear → resume
at 80%.

## Install

Ships with the autospec suite (Claude Code) — no separate installer. Get it via
the one-line bootstrap, then update with the suite:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash
```

Already installed? Re-run the suite installer with `--update`, which reinstalls every
skill in place:

```bash
./install.sh --update
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

## Hard rules

Read-only. The skill never writes files, labels, GitHub state, or log/PID files,
and tolerates a missing log gracefully (exit 0 with a clear message).
